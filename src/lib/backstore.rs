use std::marker::PhantomData;
use nix::sys::mman;
use std::mem;

#[derive(Debug, Clone)]
crate struct BackStore<T> {
    crate len: usize,
    allocated_size: usize,
    data: *mut libc::c_void,
    _phantom: PhantomData<T>
}

impl<T> BackStore<T> {
    crate fn new(len: usize) -> Result<BackStore<T>, ()> {
        // Compute the size (in bytes) needed to store the object in memory
        let mut size = 512;
        while len * mem::size_of::<T>() > size {
            size *= 2;
        }

        let backing_store = unsafe {
            // Map into memory and let backing_store point to it
            match mman::mmap(0 as *mut libc::c_void, size, mman::ProtFlags::PROT_READ | mman::ProtFlags::PROT_WRITE, mman::MapFlags::MAP_SHARED | mman::MapFlags::MAP_ANONYMOUS, -1, 0) {
                Ok(x) => x,
                Err(_) => {
                    return Err(());
                }
            }
        };
        Ok(BackStore {
            len,
            allocated_size: size,
            data: backing_store,
            _phantom: PhantomData
        })
    }

    crate fn get(&self, pos: usize) -> Option<T> {
        if pos > self.len {
            return None;
        }
		let ptr = (self.data as usize + pos * mem::size_of::<T>()) as *mut T;
		Some(unsafe { mem::transmute_copy(&*ptr) })
    }

	crate fn set(&self, pos: usize, val: T) {
		if pos > self.len {
			return;
		}
		let ptr = (self.data as usize + pos * mem::size_of::<T>()) as *mut T;
		unsafe {
			*ptr = val
		}
    }
}

impl<T> Drop for BackStore<T> {
    fn drop(&mut self) {
        unsafe {
            let _ = mman::munmap(self.data, self.allocated_size);
        }
    }
}
