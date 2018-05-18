#![feature(crate_visibility_modifier)]
extern crate nix;
extern crate libc;

use std::mem;
use std::sync::Arc;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::num::Wrapping;
use nix::sys::{mman, eventfd};
use nix::unistd;
use std::os::unix::io::RawFd;

/// The whole point of this struct is to be able to share it inside an Arc to prevent the sender
/// from being deleted while having a Reader still exists, thus leading to memory unsafety (hereby
/// be dragons !)
#[derive(Debug)]
crate struct MessageQueueInternal {
    crate len: usize,
    // eventfd to notify about availability of data
    crate fd: RawFd,
    backing_store: *mut libc::c_void,
    allocated_size: usize,
    read_pointer: AtomicUsize,
    write_pointer: AtomicUsize,
}

// this better work !
unsafe impl Send for MessageQueueInternal { }
unsafe impl Sync for MessageQueueInternal { }

impl MessageQueueInternal {
    pub fn unread(&self) -> usize {
        (Wrapping(self.write_pointer.load(Ordering::SeqCst)) - Wrapping(self.read_pointer.load(Ordering::SeqCst))).0 % self.len
    }
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

impl Drop for MessageQueueInternal {
    fn drop(&mut self) {
        unsafe {
            let _ = unistd::close(self.fd);
            let _ = mman::munmap(self.backing_store, self.allocated_size);
        }
    }
}

#[derive(Debug)]
pub struct MessageQueueSender<T> {
    crate internal: Arc<MessageQueueInternal>,
    _t: PhantomData<T>
}

#[derive(Debug)]
pub struct MessageQueueReader<T> {
    crate internal: Arc<MessageQueueInternal>,
    _t: PhantomData<T>
}

/// Create a queue.
/// This create a sender object from which you can then create readers.
impl<T: Sized> MessageQueueSender<T> {
    /// Create a new MessageQueueSender object, by specifying the number of elements it must be able to hold.
    /// The size is thus fixed at creation and cannot be changed at runtime.
    pub fn new(num_elements: usize) -> Result<MessageQueueSender<T>, MessageQueueError> {
        if num_elements < 2 {
            return Err(MessageQueueError::UnvalidSize);
        }
        // Compute the size (in bytes) needed to store the object in memory
        let mut size = 2048;
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

        let fd = match eventfd::eventfd(0, eventfd::EfdFlags::empty()) {
                Ok(x) => x,
                Err(_) => return Err(MessageQueueError::FileDescriptorCreationFailed)
        };

        let internal = MessageQueueInternal {
            len: num_elements,
            fd,
            backing_store,
            allocated_size: size,
            read_pointer: AtomicUsize::new(0),                                                                        
            write_pointer: AtomicUsize::new(0)
        };

        Ok(MessageQueueSender {
            internal: Arc::new(internal),
            _t: PhantomData
        })
    }
    /// It is not possible currently to write len values into the queue but only 'len - 1' (to be
    /// able to segregate the inital case (unread() = 0) from the full case (unread() = 'len - 1')
    pub fn send(&self, val: T) -> Result<(), MessageQueueError> {
        let wpos = self.internal.write_pointer.load(Ordering::SeqCst);
        let rpos = self.internal.read_pointer.load(Ordering::SeqCst);
        let unread = (Wrapping(wpos) - Wrapping(rpos)).0 % self.internal.len;

        if unread == self.internal.len - 1 {
            return Err(MessageQueueError::MessageQueueFull)
        }

        let ptr = (self.internal.backing_store as usize + wpos * mem::size_of::<T>()) as *mut T;
        unsafe {
            *ptr = val;
        }

        self.internal.write_pointer.store((wpos+1)%self.internal.len, Ordering::SeqCst);
        if unread == 0 {
            // In case of overflow, everything will explode (until somenoe call read on the fd) !
            // Hopefully, a reader whill call 'MessageQueueReader::read', which will in turn call read
            unistd::write(self.internal.fd, &[1, 0, 0, 0, 0, 0, 0, 0])?;
        }
        Ok(())
    }

    /// Beware, this is not thread safe to use multiple times, as a thread modifying the read
    /// pointer will modify the read pointers of all readers !
    pub fn new_reader(&self) -> MessageQueueReader<T> {
        MessageQueueReader {
            internal: self.internal.clone(),
            _t: PhantomData
        }
    }

    pub fn get_fd(&self) -> RawFd {
        self.internal.fd
    }
}

impl<T: Sized> MessageQueueReader<T> {
    /// Return number of entries available to read
    pub fn unread(&self) -> usize {
        self.internal.unread()
    }

    pub fn is_ready(&self) -> bool {
        self.internal.unread() > 0
    }

    // The 'mut' reference to 'self' is there to prevent multiple readers from accessing concurrently
    // the message queue
    pub fn read(&mut self) -> Result<T, MessageQueueError> {
        if !self.is_ready() {
            return Err(MessageQueueError::MessageQueueEmpty);
        }
        let pos = self.internal.read_pointer.load(Ordering::SeqCst);
        let ptr = (self.internal.backing_store as usize + pos * mem::size_of::<T>()) as *mut T;
        let val: T = unsafe { mem::transmute_copy(&*ptr) };
        if pos == self.internal.len - 1 {
            self.internal.read_pointer.store(0, Ordering::SeqCst);
        } else {
            self.internal.read_pointer.fetch_add(1, Ordering::SeqCst);
        }
        if !self.is_ready() {
            let mut garbage = [0; 8];
            unistd::read(self.internal.fd, &mut garbage)?;
        }
        Ok(val)
    }

    pub fn purge(&mut self) {
        self.internal.read_pointer.store(
            self.internal.write_pointer.load(Ordering::SeqCst),
            Ordering::SeqCst);
    }


    // TODO: implement blocking read

    pub fn get_fd(&self) -> RawFd {
        self.internal.fd
    }
}

/// Create a Message queue with a write and a reader.
/// This is very akin to a ruststd channel.
/// However, the whole reason of this implementation is to be able to listen on its file descriptor
/// using epoll, which was apparently not possible on channels.
pub fn MessageQueue<T>(num_elements: usize) -> Result<(MessageQueueSender<T>, MessageQueueReader<T>), MessageQueueError> {
    let sender = match MessageQueueSender::new(num_elements) {
        Ok(x) => x,
        Err(e) => return Err(e)
    };
    let reader = sender.new_reader();
    Ok((sender, reader))
}
