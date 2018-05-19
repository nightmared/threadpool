#![feature(test)]
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
    let (cmd_tx, mut _cmd_rx) = MessageQueue(25)?;
    let (_cmd_tx, mut cmd_rx) = MessageQueue(25)?;
    let tp = TP::new(_cmd_rx, _cmd_tx, 2, handler)?;
    cmd_tx.send(CmdQuery::Stop)?;
    let joinhandle = tp.run();
    loop {
        let msg = cmd_rx.blocking_read().unwrap();
        println!("{:?}", msg);
    }
    println!("{:?}", joinhandle.join());
    //tp.add_task(15);
    Ok(())
}
