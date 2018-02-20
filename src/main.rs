extern crate tokio;
extern crate futures;
extern crate tokio_io;
extern crate bytes;

use tokio::executor::current_thread;
use tokio::net::{TcpListener, TcpStream};
use tokio_io::io;
use tokio_io::{ AsyncRead, AsyncWrite};

use futures::{Future, Poll, Async, Stream};
use futures::sync::mpsc;

use bytes::BytesMut;

use std::io::{Write};
use std::sync::Arc;
use std::sync::Mutex;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::net::SocketAddr;

/// Shorthand for the transmit half of the message channel.
type Tx = mpsc::UnboundedSender<BytesMut>;

/// Shorthand for the receive half of the message channel.
type Rx = mpsc::UnboundedReceiver<BytesMut>;

type SharedState = HashMap<SocketAddr, Tx>;

struct Error;

struct ChunkData{
    socket: io::ReadHalf<TcpStream>,
    buffer: BytesMut,
    state: Rc<RefCell<SharedState>>
}

impl ChunkData{
    fn new(socket: io::ReadHalf<TcpStream>, state: Rc<RefCell<SharedState>>) -> Self {
        ChunkData{
            socket,
            buffer: BytesMut::new(),
            state
        }
    }

    fn get_chunk(&mut self) -> Poll<usize, Error> {
        self.buffer.clear();
        loop {
            self.buffer.reserve(1024);
            let result = self.socket.read_buf(&mut self.buffer);
            match result {
                Ok(Async::Ready(nb)) => return Ok(Async::Ready( nb )),
                Ok(Async::NotReady) => return Ok(Async::NotReady),
                _ => return Err(Error),
            }

        }
    }
}

impl Stream for ChunkData{
    type Item = BytesMut;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match self.get_chunk() {
            Ok(Async::Ready(0)) => return Ok(Async::Ready( None )),
            Ok(Async::Ready(_)) => return Ok(Async::Ready( Some(self.buffer.clone()) )),
            Err(_) => return Err(Error) ,
            _ => return Ok(Async::NotReady),
        }
    }
}

fn send_data_to_clients(shared: Rc<RefCell<SharedState>>, peer_addr: SocketAddr, data: BytesMut){
    shared.borrow_mut()
        .iter_mut()
        .for_each(|(&addr, send)| {
            if addr != peer_addr {
                let result = send.unbounded_send(data.clone());
                if let Err(_) = result {
                    eprintln!("warning: client no longer connected ({:?})", addr);
                }
            }
        });
}

fn main() {
    let adress = "127.0.0.1:10000".parse().unwrap();
    let listener = TcpListener::bind(&adress).unwrap();
    let shared_state: Rc<RefCell<SharedState>> = Rc::new(RefCell::new(HashMap::new()));
    let server = listener.incoming().for_each(move |socket| {
        let first_shared = shared_state.clone();
        let another_shared_state = shared_state.clone();

        let peer_addr = socket.peer_addr().unwrap();

        println!("Got connection from {}", peer_addr);

        let (sock_reader, sock_writer) = socket.split();

        let (send_to_this_client, recv_for_client_sending) = mpsc::unbounded::<BytesMut>();

        let client_resolve_future = recv_for_client_sending.fold(sock_writer, |sock_writer, bytes_received_to_send| {
            io::write_all(sock_writer, bytes_received_to_send)
                .map(|(writer, _bytes)| writer)
                .map_err(|io_error| eprintln!("warning: io error: {}", io_error))
        }).then(|_result| {
            Ok(())
        });

        current_thread::spawn(client_resolve_future); // handle IO in the background.

        shared_state.borrow_mut().insert(peer_addr, send_to_this_client);
        let chunk_data = ChunkData::new(sock_reader, first_shared);
        let mut chuck_data = chunk_data.for_each(move |data| {
                                    println!("{:?}", data);
                                    send_data_to_clients(another_shared_state.clone(), peer_addr, data);
                                    Ok( () )
                                }).map_err(|_| ());


        current_thread::spawn(chuck_data);

        Ok(())
    }).map_err(|_| ());

    current_thread::run(|_| {
        current_thread::spawn(server);
        println!("Server running on port 10000");
    })

}