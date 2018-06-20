extern crate test;
use lib::messagequeue::*;
use std::thread;
use std::time::{Duration, SystemTime};

#[derive(Debug, PartialEq)]
struct TestStruct {
    a: usize,
    b: String,
    c: [usize; 2]
}

fn send_msg(tx: &mut MessageQueueSender<usize>, num: usize) {
    for i in 0..num {
        assert!(tx.send(i).is_ok());
    }
}

#[test]
fn create_message_queue() {
    assert_eq!(MessageQueueSender::<usize>::new(0).err(), Some(MessageQueueError::UnvalidSize));
    assert_eq!(MessageQueueSender::<usize>::new(1).err(), Some(MessageQueueError::UnvalidSize));
    // Attempt to create a queue to contain 10^12 messages
    // This shouldn't work until someone with much more money than myself decided to use it (or the
    // kernel fucked us in our back) ;)
    assert_eq!(MessageQueueSender::<usize>::new(1000000000000).err(), Some(MessageQueueError::MmapFailed));

    assert!(MessageQueueSender::<&u8>::new(2048).is_ok());
    assert!(MessageQueueSender::<f64>::new(250000).is_ok());
    assert!(MessageQueueSender::<Vec<String>>::new(250000).is_ok());
    assert!(MessageQueueSender::<TestStruct>::new(250000).is_ok());
}

#[test]
fn create_reader() {
    let mut t = MessageQueueSender::<usize>::new(256).unwrap();
    let reader = t.new_reader();
    assert_eq!(reader.unread(), 0);
    assert_eq!(reader.is_ready(), false);
}

#[test]
fn send_without_reader() {
    let (mut tx, _) = MessageQueue(256).unwrap();
    send_msg(&mut tx, 256);
    // One too much
    assert_eq!(tx.send(256).err(), Some(MessageQueueError::MessageQueueFull));
}

#[test]
fn send_with_reader() {
    let (mut tx, mut rx) = MessageQueue(256).unwrap();
    send_msg(&mut tx, 127);
    assert_eq!(rx.unread(), 127);
    assert!(rx.is_ready());
    for c in 0..127  {
        assert_eq!(rx.is_ready(), true);
        assert_eq!(rx.read(), Some(c));
    }
    assert_eq!(rx.unread(), 0);
    assert!(!rx.is_ready());

    send_msg(&mut tx, 256);
    // One too much
    assert_eq!(tx.send(256).err(), Some(MessageQueueError::MessageQueueFull));

    let mut c = 0;
    while rx.is_ready() {
        assert_eq!(rx.blocking_read(), Some(c));
        c += 1;
    }
    assert_eq!(c, 256);
}

#[test]
fn send_struct() {
    let mut t = MessageQueueSender::<TestStruct>::new(256).unwrap();
    for i in 0..127 {
        t.send(TestStruct {
            a: i,
            b: "42".into(),
            c: [i, i+1]
        });
    }
    let mut r = t.new_reader();
    for i in 0..127 {
        assert_eq!(r.read(), Some(TestStruct {
            a: i,
            b: "42".into(),
            c: [i, i+1]
        }));
    }
}

#[test]
fn send_across_thread() {
    let (mut tx, mut rx) = MessageQueue(256).unwrap();
    for i in 0..127 {
        assert!(tx.send(i).is_ok());
    }

    assert!(thread::spawn(move || {
        assert_eq!(rx.unread(), 127);
        assert!(rx.is_ready());
        for c in 0..127  {
            assert_eq!(rx.is_ready(), true);
            assert_eq!(rx.read(), Some(c));
        }
        assert_eq!(rx.unread(), 0);
        assert!(!rx.is_ready());
    }).join().is_ok());
}

#[test]
fn send_concurrently() {
    let (mut tx, mut rx) = MessageQueue(8192).unwrap();
    for i in 0..4096 {
        assert!(tx.send(i).is_ok());
    }

    assert!(thread::spawn(move || {
        assert_eq!(rx.unread(), 4096);
        assert!(rx.is_ready());
        for c in 0..4096  {
            assert_eq!(rx.is_ready(), true);
            assert_eq!(rx.read(), Some(c));
        }
    }).join().is_ok());

    let (mut tx, mut rx) = MessageQueue(8192).unwrap();
    let rx2 = rx.clone();
    let sender_thread = thread::spawn(move || {
        for i in 0..8192 {
            assert!(tx.send(i).is_ok());
        }
        // yay, a spinlock ;(
        while rx2.unread() == 8192 {}
		tx.send(8888).unwrap();
    });

   let receiver_thread = thread::spawn(move || {
        let mut c = 0;
        while c < 8192 {
            if rx.is_ready() {
                assert_eq!(rx.read(), Some(c));
                c += 1;
            }
        }
        while !rx.is_ready() { }
        assert_eq!(rx.read(), Some(8888));
    });

   assert!(sender_thread.join().is_ok());
   assert!(receiver_thread.join().is_ok());
}

#[test]
fn send_concurrently_blocking_read() {
    let (mut tx, mut rx) = MessageQueue(8192).unwrap();
    let mut rx2 = rx.clone();
    for i in 0..4096 {
        assert!(tx.send(i).is_ok());
    }

    assert!(thread::spawn(move || {
        for c in 0..4096  {
            assert_eq!(rx.blocking_read(), Some(c));
        }
    }).join().is_ok());

    let now = SystemTime::now();
    let blocking_thread = thread::spawn(move || {
        assert_eq!(rx2.blocking_read(), Some(42));
        assert!(now.elapsed().unwrap() > Duration::from_millis(50));
    });

    thread::sleep(Duration::from_millis(50));
    tx.send(42).unwrap();
    assert!(blocking_thread.join().is_ok());
}

#[bench]
fn create_message_queue_struct_50(b: &mut test::Bencher) {
    b.iter(|| MessageQueueSender::<TestStruct>::new(50).unwrap());
}

#[bench]
fn create_message_queue_struct_2048(b: &mut test::Bencher) {
    b.iter(|| MessageQueueSender::<TestStruct>::new(2048).unwrap());
}

#[bench]
fn create_message_queue_struct_10m(b: &mut test::Bencher) {
    b.iter(|| MessageQueueSender::<TestStruct>::new(10000000).unwrap());
}

#[bench]
fn create_reader_2048(b: &mut test::Bencher) {
    let mut s = MessageQueueSender::<usize>::new(2048).unwrap();
    b.iter(|| s.new_reader());
}

#[bench]
fn send_1000_messages(b: &mut test::Bencher) {
    let (mut tx, mut rx) = MessageQueue(2048).unwrap();
	b.iter(|| {
        for i in 0..1000 {
            tx.send(i).unwrap();
            rx.read().unwrap();
        }
	});
}

#[bench]
fn send_1M_message_parallel(b: &mut test::Bencher) {
    let (mut tx, mut rx) = MessageQueue(2000000).unwrap();
    b.iter(|| {
        let mut rx2 = rx.clone();
        let th = thread::spawn(move || for _ in 0..1000000 {
            rx2.blocking_read().unwrap();
        });
        for i in 0..1000000 {
            tx.send(i).unwrap();
        }
        th.join().unwrap();
    });
}
