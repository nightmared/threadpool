extern crate test;
use std::{io, thread};
use std::time::Duration;
use lib::threadpool::*;

fn test_func(x: usize) -> Result<usize, io::Error> {
    Ok(x+1)
}

fn slow_func(_: usize) -> Result<usize, io::Error> {
    loop {
        thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn create() {
    assert!(PoolHandler::new(1, 2048, 2048, &test_func).is_ok());
    assert!(PoolHandler::new(2, 2048, 2048, &test_func).is_ok());
    assert!(PoolHandler::new(64, 4096, 8192, &test_func).is_ok());
}

#[test]
fn exchange_messages() {
    let mut tp = PoolHandler::new(4, 1024, 1024, &test_func).unwrap();
    assert!(tp.run(0, 42).is_ok());
    let msg = tp.blocking_read().unwrap();
    assert!(msg.id == 0);
    assert!(msg.val.unwrap().unwrap() == 43);
}

#[bench]
fn exchange_10k_messages(b: &mut test::Bencher) {
    let mut tp = PoolHandler::new(4, 20000, 50, &test_func).unwrap();
    b.iter(|| {
        for i in 0..10000 {
            assert!(tp.run(i, 2*i).is_ok());
        }
        for _ in 0..10000 {
            assert!(tp.blocking_read().is_some());
        }
    });
}
