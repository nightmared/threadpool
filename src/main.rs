#![feature(test, extern_prelude, crate_visibility_modifier)]
extern crate nix;
extern crate libc;
extern crate test;

mod lib;
#[cfg(test)] mod tests;

use std::net::{TcpListener, TcpStream, SocketAddr, Shutdown};

use lib::messagequeue::*;
use lib::threadpool::*;

//fn handler(socket: TcpStream) -> Result<(), TPError> {
//    println!("{}", socket.peer_addr().unwrap());
//    socket.shutdown(Shutdown::Both).unwrap();
//    Ok(())
//}

fn main() {
    //let mut tp = TPHandler::new(2, handler)?;
    //let listener = TcpListener::bind("127.0.0.1:8080").unwrap();
    //loop {
    //    match listener.accept() {
    //        Ok((socket, _)) => tp.send(socket).unwrap(),
    //        Err(e) => println!("couldn't get client: {:?}", e),
    //    }
    //}
}
