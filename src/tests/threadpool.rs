extern crate test;
use lib::threadpool::*;

#[test]
fn create_threadpool() {
    assert!(TPHandler::new(1, |x: usize| Ok(x+1)).is_ok());
    assert!(TPHandler::new(2, |x: usize| Ok(x+1)).is_ok());
    assert!(TPHandler::new(64, |x: usize| Ok(x+1)).is_ok());
}

#[test]
fn exchange_messages() {
    let mut tp = TPHandler::new(4, |x: usize| Ok(x+1)).unwrap();
    assert!(tp.send(42).is_ok());
    assert_eq!(tp.blocking_read(), Ok(CmdAnswer::TaskSuccess(43)));
}

#[bench]
fn exchange_10k_messages(b: &mut test::Bencher) {
    let mut tp = TPHandler::new(4, |x: usize| Ok(x+1)).unwrap();
    b.iter(|| {
        for i in 0..10000 {
            assert!(tp.send(i).is_ok());
        }
        for i in 0..10000 {
            assert!(tp.blocking_read().is_ok());
        }
    });
}
