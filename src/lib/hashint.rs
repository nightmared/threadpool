use lib::backstore::BackStore;
use std::mem;

#[derive(Debug, PartialEq)]
pub enum HashError {
	MmapFailed,
    NotFound,
	Full
}

#[derive(Clone, Debug)]
struct HashVal<T> {
	id: usize,
	val: T
}

/// Yet another dumb hash implementation
pub struct HashInt<T> {
    backing_store: BackStore<Option<HashVal<T>>>
}

impl<T: Clone> HashInt<T> {
	// take self to be able to use multiple hash functions later (the function would then be a type parameter of HashInt)
	fn hash_func(&self, id: usize) -> usize {
		id%self.backing_store.len
	}

	pub fn new(init_len: usize) -> Result<HashInt<T>, HashError> {
        match BackStore::new(init_len) {
            Ok(x) => Ok(HashInt { backing_store: x }),
            Err(()) => Err(HashError::MmapFailed)
        }
	}

	pub fn resize(&mut self, new_len: usize) -> Result<(), HashError> {
		if new_len <= self.backing_store.len {
			return Ok(());
		}
		let mut old_store = match BackStore::new(new_len) {
            Ok(x) => x,
            Err(()) => return Err(HashError::MmapFailed)
        };

		mem::swap(&mut self.backing_store, &mut old_store);
		
		for i in 0..old_store.len {
			if let Some(x) = old_store.get(i) {
				let id = self.hash_func(x.id);
				self.insert_val(id, x)?;
			}
		}
		Ok(())
	}

	// insert at given position inside the backing store, id is the hashed value
	fn insert_val(&mut self, pos: usize, val: HashVal<T>) -> Result<(), HashError> {
		for i in 0..self.backing_store.len {
			let content = self.backing_store.get((pos+i)%self.backing_store.len);
			match content {
				None => {
					self.backing_store.set((pos+i)%self.backing_store.len, Some(val));
					return Ok(());
				},
				Some(x) => {
					if x.id == val.id {
						self.backing_store.set((pos+i)%self.backing_store.len, Some(val));
						return Ok(());
					}
				}
			}
		}
		Err(HashError::Full)
	}

    pub fn get_pos(&self, id: usize) -> Option<usize> {
        let pos = self.hash_func(id);
		for i in 0..self.backing_store.len {
			let content = self.backing_store.get((pos+i)%self.backing_store.len);
			match content {
				None => return None,
				Some(x) => {
					if x.id == id {
						return Some((pos+i)%self.backing_store.len);
					}
				}
			}
		}
		None

    }

	pub fn get(&self, id: usize) -> Option<T> {
        let pos = self.get_pos(id);
        match pos {
            Some(x) => Some(self.backing_store.get(x).unwrap().val),
            None => None
        }
	}

	pub fn insert(&mut self, id: usize, val: T) -> Result<(), HashError> {
		let pos = self.hash_func(id);
		self.insert_val(pos, HashVal{ id, val })
	}

    pub fn remove(&mut self, id: usize) -> Result<(), HashError> {
        let mut pos = match self.get_pos(id) {
            Some(x) => x,
            None => return Err(HashError::NotFound)
        };
        self.backing_store.set(pos, None);
        // move values that were "pushed forward"
        while let Some(x) = self.backing_store.get((pos+1)%self.backing_store.len) {
            let expected_pos = self.hash_func(x.id);
            if (pos+1-expected_pos)%self.backing_store.len == 0 {
                return Ok(());
            }
            self.backing_store.set(pos, self.backing_store.get((pos+1)%self.backing_store.len));
            self.backing_store.set((pos+1)%self.backing_store.len, None);
            pos = (pos+1)%self.backing_store.len;
        }
        Ok(())
    }
}
