use std::mem;
use std::sync::Arc;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::num::Wrapping;
use nix::sys::{mman, eventfd};
use std::thread;
use nix::unistd;
use std::os::unix::io::RawFd;

/// The whole point of this struct is to be able to share it inside an Arc to prevent the sender
/// from being deleted while having a Reader still exists, thus leading to memory unsafety (hereby
/// be dragons !)
#[derive(Debug)]
pub(crate) struct MessageQueueInternal {
    pub len: usize,
    allocated_size: usize,
    available: AtomicUsize,
	read_ptr: AtomicUsize,
    backing_store: *mut libc::c_void
}

// this better work !
unsafe impl Send for MessageQueueInternal { }
unsafe impl Sync for MessageQueueInternal { }

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

impl Drop for MessageQueueInternal {
    fn drop(&mut self) {
        unsafe {
            let _ = mman::munmap(self.backing_store, self.allocated_size);
        }
    }
}

#[derive(Debug)]
pub struct MessageQueueSender<T> {
    crate internal: Arc<MessageQueueInternal>,
    write_pointer: usize,
    _t: PhantomData<T>
}

#[derive(Debug, Clone)]
pub struct MessageQueueReader<T> {
    crate internal: Arc<MessageQueueInternal>,
    _t: PhantomData<T>
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
        // Compute the size (in bytes) needed to store the object in memory
        let mut size = 512;
        while num_elements * mem::size_of::<T>() > size {
            size *= 2;
        }
        
        let backing_store = unsafe {
            // Map into memory and let backing_store point to it
            match mman::mmap(0 as *mut libc::c_void, size, mman::ProtFlags::PROT_READ | mman::ProtFlags::PROT_WRITE, mman::MapFlags::MAP_SHARED | mman::MapFlags::MAP_ANONYMOUS, -1, 0) {
                Ok(x) => x,
                Err(_) => {
                    return Err(MessageQueueError::MmapFailed);
                }
            }
        };

        let internal = MessageQueueInternal {
            len: num_elements,
            available: AtomicUsize::new(0),
            backing_store,
			read_ptr: AtomicUsize::new(0),
            allocated_size: size
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

        let wpos = self.write_pointer;
        let ptr = (self.internal.backing_store as usize + wpos * mem::size_of::<T>()) as *mut T;
        unsafe {
            *ptr = val;
        }
        self.write_pointer = (wpos+1)%self.internal.len;
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
        let ptr = (self.internal.backing_store as usize + rpos * mem::size_of::<T>()) as *mut T;
        let val: T = unsafe { mem::transmute_copy(&*ptr) };
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
