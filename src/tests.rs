use lib::*;
use std::thread;

struct TestStruct {
    a: usize,
    b: u32,
    c: [u8; 6]
}

#[test]
fn create_message_queue() {
    let t = MessageQueueSender::<usize>::new(0);
    assert_eq!(t.err(), Some(MessageQueueError::UnvalidSize));
    let t = MessageQueueSender::<&u8>::new(2048);
    assert!(t.is_ok());
    for i in 0..5 {
        let t = MessageQueueSender::<f64>::new(25*10^i);
        assert!(t.is_ok());
        let t = MessageQueueSender::<Vec<String>>::new(25*10^i);
        assert!(t.is_ok());
        let t = MessageQueueSender::<TestStruct>::new(25*10^i);
        assert!(t.is_ok());
    }
}

#[test]
fn create_reader() {
    let t = MessageQueueSender::<usize>::new(256).unwrap();
    let reader = t.new_reader();
    assert_eq!(reader.available(), 0);
    assert_eq!(reader.is_ready(), false);
}

#[test]
fn send_without_reader() {
    let mut t = MessageQueueSender::<usize>::new(256).unwrap();
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
    let mut t = MessageQueueSender::<usize>::new(256).unwrap();
    for i in 0..127 {
        let res = t.send(i);
        assert!(res.is_ok());
    }
    let mut reader = t.new_reader();
    assert_eq!(reader.available(), 127);
    assert!(reader.is_ready());
    for c in 0..127  {
        assert_eq!(reader.is_ready(), true);
        assert_eq!(reader.read(), Ok(c));
    }
    assert_eq!(reader.available(), 0);
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
        assert_eq!(reader.read(), Ok(c));
        c += 1;
    }
    assert_eq!(c, 255);
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
        assert_eq!(reader.available(), 127);
        assert!(reader.is_ready());
        for c in 0..127  {
            assert_eq!(reader.is_ready(), true);
            assert_eq!(reader.read(), Ok(c));
        }
    }).join().is_ok());

    let mut reader = t.new_reader();
    assert!(thread::spawn(move || {
        assert_eq!(reader.available(), 0);
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
    let mut t = MessageQueueSender::<usize>::new(8192).unwrap();
    for i in 0..4096 {
        let res = t.send(i);
        assert!(res.is_ok());
    }

    let mut reader = t.new_reader();
    assert!(thread::spawn(move || {
        assert_eq!(reader.available(), 4096);
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
        t.send(8888);
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
