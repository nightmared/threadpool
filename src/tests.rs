extern crate test;
use lib::*;
use std::thread;
use std::time::{Duration, SystemTime};

#[repr(packed)]
#[derive(Debug, PartialEq)]
struct TestStruct {
    a: usize,
    b: String,
    c: [usize; 2]
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
    for i in 0..5 {
        assert!(MessageQueueSender::<f64>::new(25*10^i).is_ok());
        assert!(MessageQueueSender::<Vec<String>>::new(25*10^i).is_ok());
        assert!(MessageQueueSender::<TestStruct>::new(25*10^i).is_ok());
    }
}

#[test]
fn create_reader() {
    let t = MessageQueueSender::<usize>::new(256).unwrap();
    let reader = t.new_reader();
    assert_eq!(reader.unread(), 0);
    assert_eq!(reader.is_ready(), false);
}

#[test]
fn send_without_reader() {
    let t = MessageQueueSender::<usize>::new(256).unwrap();
    for i in 0..255 {
        let res = t.send(i);
        assert!(res.is_ok());
    }
    // One too much
    let res = t.send(256);
    assert_eq!(res.err(), Some(MessageQueueError::MessageQueueFull));
}

#[test]
fn send_with_reader() {
    let t = MessageQueueSender::<usize>::new(256).unwrap();
    for i in 0..127 {
        let res = t.send(i);
        assert!(res.is_ok());
    }
    let mut reader = t.new_reader();
    assert_eq!(reader.unread(), 127);
    assert!(reader.is_ready());
    for c in 0..127  {
        assert_eq!(reader.is_ready(), true);
        assert_eq!(reader.read(), Ok(c));
    }
    assert_eq!(reader.unread(), 0);
    assert!(!reader.is_ready());

    for i in 0..255 {
        let res = t.send(i);
        assert!(res.is_ok());
    }

    // One too much
    let res = t.send(256);
    assert_eq!(res.err(), Some(MessageQueueError::MessageQueueFull));

    let mut c = 0;
    while reader.is_ready() {
        assert_eq!(reader.blocking_read(), Ok(c));
        c += 1;
    }
    assert_eq!(c, 255);
}

#[test]
fn send_struct() {
    let t = MessageQueueSender::<TestStruct>::new(256).unwrap();
    for i in 0..127 {
        t.send(TestStruct {
            a: i,
            b: "42".into(),
            c: [i, i+1]
        });
    }
    let mut r = t.new_reader();
    for i in 0..127 {
        assert_eq!(r.read(), Ok(TestStruct {
            a: i,
            b: "42".into(),
            c: [i, i+1]
        }));
    }
}

#[test]
fn send_across_thread() {
    let t = MessageQueueSender::<usize>::new(256).unwrap();
    for i in 0..127 {
        let res = t.send(i);
        assert!(res.is_ok());
    }

    let mut reader = t.new_reader();
    assert!(thread::spawn(move || {
        assert_eq!(reader.unread(), 127);
        assert!(reader.is_ready());
        for c in 0..127  {
            assert_eq!(reader.is_ready(), true);
            assert_eq!(reader.read(), Ok(c));
        }
    }).join().is_ok());

    let reader = t.new_reader();
    assert!(thread::spawn(move || {
        assert_eq!(reader.unread(), 0);
        assert!(!reader.is_ready());
    }).join().is_ok());

    let mut reader = t.new_reader();
    // Ensure the data is not destroyed if 't' goes out of scope (basically, if the Arc works as
    // I'm expecting it to)
    assert!(thread::spawn(move || {
        for i in 0..255 {
            let res = t.send(i);
            assert!(res.is_ok());
        }

        // One too much
        let res = t.send(256);
        assert_eq!(res.err(), Some(MessageQueueError::MessageQueueFull));
    }).join().is_ok());

    assert!(thread::spawn(move || {
        for i in 0..255 {
            assert!(reader.is_ready());
            assert_eq!(reader.read(), Ok(i));
        }
        assert!(!reader.is_ready());
    }).join().is_ok());
}

#[test]
fn send_concurrently() {
    let t = MessageQueueSender::<usize>::new(8192).unwrap();
    for i in 0..4096 {
        let res = t.send(i);
        assert!(res.is_ok());
    }

    let mut reader = t.new_reader();
    assert!(thread::spawn(move || {
        assert_eq!(reader.unread(), 4096);
        assert!(reader.is_ready());
        for c in 0..4096  {
            assert_eq!(reader.is_ready(), true);
            assert_eq!(reader.read(), Ok(c));
        }
    }).join().is_ok());

    let mut reader = t.new_reader();
    let sender_thread = thread::spawn(move || {
        for i in 0..8192 {
            let res = t.send(i);
            assert!(res.is_ok());
        }
        // Yeah, horrible spinlocks ;)
        while t.internal.unread() != 0 { }
        t.send(8888).unwrap();
    });

   let receiver_thread = thread::spawn(move || {
        let mut c = 0;
        while c < 8192 {
            if reader.is_ready() {
                assert_eq!(reader.read(), Ok(c));
                c += 1;
            }
        }
        while !reader.is_ready() { }
        assert_eq!(reader.read(), Ok(8888));
    });

   assert!(sender_thread.join().is_ok());
   assert!(receiver_thread.join().is_ok());
}

#[test]
fn send_concurrently_blocking_read() {
    let t = MessageQueueSender::<usize>::new(8192).unwrap();
    for i in 0..4096 {
        let res = t.send(i);
        assert!(res.is_ok());
    }

    let mut reader = t.new_reader();
    assert!(thread::spawn(move || {
        for c in 0..4096  {
            assert_eq!(reader.blocking_read(), Ok(c));
        }
    }).join().is_ok());

    let mut reader = t.new_reader();
    let blocking_thread = thread::spawn(move || {
        let now = SystemTime::now();
        assert_eq!(reader.blocking_read(), Ok(42));
        assert!(now.elapsed().unwrap() > Duration::from_millis(50));
    });

    thread::sleep(Duration::from_millis(50));
    t.send(42).unwrap();
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
    let s = MessageQueueSender::<usize>::new(2048).unwrap();
    b.iter(|| s.new_reader());
}

#[bench]
fn send_message_2048(b: &mut test::Bencher) {
    let s = MessageQueueSender::<usize>::new(2048).unwrap();
	let mut r = s.new_reader();
	b.iter(|| {
        for i in 0..1000 {
            s.send(i).unwrap();
            r.read().unwrap();
        }
	});
}
