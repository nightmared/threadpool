use std::marker::PhantomData;
use nix::sys::mman;
use std::mem;

#[derive(Debug, Clone)]
crate struct BackStore<T> {
    crate len: usize,
    data: *mut libc::c_void,
    _phantom: PhantomData<T>
}

unsafe impl<T> Send for BackStore<T> {}

impl<T> BackStore<T> {
    crate fn new(len: usize) -> Result<BackStore<T>, ()> {
        let backing_store = unsafe {
            // Map into memory and let backing_store point to it
            match mman::mmap(0 as *mut libc::c_void, len*mem::size_of::<T>(), mman::ProtFlags::PROT_READ | mman::ProtFlags::PROT_WRITE, mman::MapFlags::MAP_SHARED | mman::MapFlags::MAP_ANONYMOUS, -1, 0) {
                Ok(x) => x,
                Err(_) => {
                    return Err(());
                }
            }
        };
        Ok(BackStore {
            len,
            data: backing_store,
            _phantom: PhantomData
        })
    }

    // Beware of being within bounds, no checks will be done
    #[inline]
    crate fn get(&self, pos: usize) -> T {
        let ptr = (self.data as usize + pos * mem::size_of::<T>()) as *mut T;
        unsafe { mem::transmute_copy(&*ptr) }
    }

    #[inline]
    crate fn set(&self, pos: usize, val: T) {
        let ptr = (self.data as usize + pos * mem::size_of::<T>()) as *mut T;
        unsafe {
            *ptr = val
        }
    }
}

impl<T> Drop for BackStore<T> {
    fn drop(&mut self) {
        unsafe {
            let _ = mman::munmap(self.data, self.len*mem::size_of::<T>());
        }
    }
}
