use lib::backstore::BackStore;
use std::mem;

#[derive(Debug, PartialEq)]
pub enum HashError {
	MmapFailed,
	Full
}

#[derive(Clone)]
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
			if let Some(x) = old_store.get(i).unwrap() {
				let id = self.hash_func(x.id);
				self.insert_val(id, x)?;
			}
		}
		Ok(())
	}

	// insert at given position inside the backing store, id is the hashed value
	fn insert_val(&mut self, pos: usize, val: HashVal<T>) -> Result<(), HashError> {
		for i in 0..self.backing_store.len {
			if self.backing_store.get((pos+i)%self.backing_store.len).unwrap().is_none() {
				self.backing_store.set((pos+i)%self.backing_store.len, Some(val));
				return Ok(());
			}
		}
		Err(HashError::Full)
	}

	pub fn get(&self, id: usize) -> Option<T> {
		let pos = self.hash_func(id);
		for i in 0..self.backing_store.len {
			let content = self.backing_store.get((pos+i)%self.backing_store.len).unwrap();
			match content {
				None => return None,
				Some(x) => {
					if x.id == id {
						return Some(x.val);
					}
				}
			}
		}
		None
	}

	pub fn insert(&mut self, id: usize, val: T) -> Result<(), HashError> {
		let pos = self.hash_func(id);
		self.insert_val(pos, HashVal{ id, val })
	}
}
