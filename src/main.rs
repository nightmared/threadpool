#![feature(test, try_trait)]
extern crate nix;
extern crate libc;

mod lib;
#[cfg(test)] mod tests;

use lib::messagequeue::*;
use lib::threadpool::*;

fn handler(x: usize) -> Result<usize, TPError> {
    Ok(x+1)
}

fn main() -> Result<(), TPError> {
    let tp = TPHandler::new(2, handler)?;
    for i in 0..25 {
        tp.send(i)?;
    }
    tp.stop(None)?;
    loop {
        let msg = cmd_rx.blocking_read().unwrap();
        println!("{:?}", msg);
    }
    println!("{:?}", joinhandle.join());
    //tp.add_task(15);
    Ok(())
}
