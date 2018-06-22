use std::sync::Arc;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{io, thread};
use lib::backstore::BackStore;

/// The whole point of this struct is to be able to share it inside an Arc to prevent the sender
/// from being deleted while having a Reader still exists, thus leading to memory unsafety (hereby
/// be dragons !)
#[derive(Debug)]
pub(crate) struct MessageQueueInternal<T> {
    pub len: usize,
    available: AtomicUsize,
    read_ptr: AtomicUsize,
    backing_store: BackStore<T>
}

// this better work !
unsafe impl<T> Send for MessageQueueInternal<T> { }
unsafe impl<T> Sync for MessageQueueInternal<T> { }

#[derive(Debug)]
pub struct MessageQueueSender<T> {
    crate internal: Arc<MessageQueueInternal<T>>,
    write_pointer: usize,
    _t: PhantomData<T>
}

#[derive(Debug, Clone)]
pub struct MessageQueueReader<T> {
    crate internal: Arc<MessageQueueInternal<T>>,
    _t: PhantomData<T>
}

#[derive(Debug, PartialEq)]
pub enum MessageQueueError {
    UnvalidSize,
    FileDescriptorCreationFailed,
    MemoryAllocationFailed,
    MmapFailed,
    MessageSendingFailed,
    MessageQueueFull,
    MessageQueueEmpty,
    NixError(nix::Error)
}

impl From<nix::Error> for MessageQueueError {
    fn from(e: nix::Error) -> Self {
        MessageQueueError::NixError(e)
    }
}

impl From<MessageQueueError> for io::Error {
    fn from(e: MessageQueueError) -> Self {
        io::Error::new(io::ErrorKind::Other, "MessageQueueError")
    }
}

/// Create a queue.
/// This create a sender object from which you can then create readers.
impl<T: Sized> MessageQueueSender<T> {
    /// Create a new MessageQueueSender object, by specifying the number of elements it must be able to hold.
    /// The size is thus fixed at creation and cannot be changed at runtime.
    crate fn new(num_elements: usize) -> Result<MessageQueueSender<T>, MessageQueueError> {
        if num_elements < 2 {
            return Err(MessageQueueError::UnvalidSize);
        }

        let backing_store = match BackStore::new(num_elements) {
            Ok(x) => x,
            Err(()) => return Err(MessageQueueError::MmapFailed)
        };

        let internal = MessageQueueInternal {
            len: num_elements,
            available: AtomicUsize::new(0),
            read_ptr: AtomicUsize::new(0),
            backing_store
        };

        Ok(MessageQueueSender {
            internal: Arc::new(internal),
            write_pointer: 0,
            _t: PhantomData
        })
    }

    /// Send a message to the queue
    pub fn send(&mut self, val: T) -> Result<(), MessageQueueError> {
        if self.internal.available.load(Ordering::SeqCst) == self.internal.len {
            return Err(MessageQueueError::MessageQueueFull);
        }

        self.internal.backing_store.set(self.write_pointer, val);

        self.write_pointer = (self.write_pointer+1)%self.internal.len;
        self.internal.available.fetch_add(1, Ordering::SeqCst);

        Ok(())
    }

    crate fn new_reader(&mut self) -> MessageQueueReader<T> {
        MessageQueueReader {
            internal: self.internal.clone(),
            _t: PhantomData
        }
    }
}

impl<T: Sized> MessageQueueReader<T> {
    /// Return number of entries available to read
    pub fn unread(&self) -> usize {
        self.internal.available.load(Ordering::SeqCst)
    }

    pub fn is_ready(&self) -> bool {
        self.unread() > 0
    }

    /// Get current value pointed to by the read_pointer and update the read_pointer.
    fn get_current_val(&mut self) -> T {
        let rpos = self.internal.read_ptr.load(Ordering::SeqCst)%self.internal.len;

        let val = self.internal.backing_store.get(rpos);

        self.internal.available.fetch_sub(1, Ordering::SeqCst);
        self.internal.read_ptr.fetch_add(1, Ordering::SeqCst);
        val
    }

    pub fn read(&mut self) -> Option<T> {
        if self.unread() == 0 {
            None
        } else {
            Some(self.get_current_val())
        }
    }

    pub fn blocking_read(&mut self) -> Option<T> {    
        while self.unread() == 0 {
            thread::yield_now();
        }

        Some(self.get_current_val())
    }
}

/// Create a Message queue with a sender and a reader.
/// This is very akin to a ruststd channel.
/// However, the whole reason of this implementation is to be able to listen on its file descriptor
/// using epoll, which was apparently not possible on channels.
pub fn MessageQueue<T>(num_elements: usize) -> Result<(MessageQueueSender<T>, MessageQueueReader<T>), MessageQueueError> {
    let mut sender = match MessageQueueSender::new(num_elements) {
        Ok(x) => x,
        Err(e) => return Err(e)
    };
    let reader = sender.new_reader();
    Ok((sender, reader))
}
