#![feature(crate_visibility_modifier)]
extern crate nix;
extern crate libc;

use std::mem;
use std::sync::Arc;
use std::marker::PhantomData;
use std::ffi::CStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::num::Wrapping;
use nix::sys::{memfd, mman};
use nix::unistd;
use std::os::unix::io::RawFd;

/// The whole point of this struct is to be able to share it inside an Arc to prevent the sender
/// from being deleted while having a Reader still exists, thus leading to memory unsafety (hereby
/// be dragons !)
#[derive(Debug)]
crate struct MessageQueueInternal {
    len: usize,
    fd: RawFd,
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

    pub fn get_fd(&self) -> RawFd {
        self.fd
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
    MessageQueueEmpty
}

impl Drop for MessageQueueInternal {
    fn drop(&mut self) {
        unsafe {
            mman::munmap(self.backing_store, self.allocated_size);
            let _ = unistd::close(self.fd);
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
        
        let (fd, backing_store) = unsafe {
            // TODO: investigate whether HugePages is worth it
            let tmp = memfd::memfd_create(&CStr::from_bytes_with_nul(b"MessageQueue\0").unwrap(), memfd::MemFdCreateFlag::empty());
            if tmp.is_err() {
                return Err(MessageQueueError::FileDescriptorCreationFailed);
            }
            let fd = tmp.unwrap();

            match nix::unistd::ftruncate(fd, size as libc::off_t) {
                Ok(()) => {},
                Err(_) => {
                    let _ = unistd::close(fd);
                    return Err(MessageQueueError::MemoryAllocationFailed);
                }
            };
            // Map into memory and let backing_store point to it
            // TODO: if hugepages are used, take care of adding the MAP_HUGETLB flag !
            let addr = match mman::mmap(0 as *mut libc::c_void, size, mman::ProtFlags::PROT_READ | mman::ProtFlags::PROT_WRITE, mman::MapFlags::MAP_SHARED, fd, 0) {
                Ok(x) => x,
                Err(_) => {
                    let _ = unistd::close(fd);
                    return Err(MessageQueueError::MmapFailed);
                }
            };
            (fd, addr)
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
        if self.internal.unread() == self.internal.len - 1 {
            return Err(MessageQueueError::MessageQueueFull)
        }
        let pos = self.internal.write_pointer.load(Ordering::SeqCst);
        let ptr = (self.internal.backing_store as usize + pos * mem::size_of::<T>()) as *mut T;
        unsafe {
            *ptr = val;
        }
        if pos == self.internal.len - 1 {
            self.internal.write_pointer.store(0, Ordering::SeqCst);
        } else {
            self.internal.write_pointer.fetch_add(1, Ordering::SeqCst);
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
        self.internal.get_fd()
    }
}

impl<T: Sized> MessageQueueReader<T> {
    pub fn available(&self) -> usize {
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
        Ok(val)
    }

    // TODO: implement blocking read

    pub fn get_fd(&self) -> RawFd {
        self.internal.get_fd()
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
