use lib::hashint::*;

#[test]
fn create() {
	assert!(HashInt::<usize>::new(256).is_ok());
	assert!(HashInt::<f64>::new(512).is_ok());
	assert!(HashInt::<String>::new(1024).is_ok());
	assert!(HashInt::<u8>::new(2048).is_ok());
}

#[test]
fn insert() {
	let mut map = HashInt::new(256).unwrap();
	for i in 0..150 {
		assert!(map.insert(5*i, i).is_ok());
	}
	for i in 0..150 {
		assert_eq!(map.get(5*i), Some(i));
	}
}

#[test]
fn resize() {
	let mut map = HashInt::new(256).unwrap();
	for i in 0..150 {
		assert!(map.insert(5*i, i).is_ok());
	}
	assert!(map.resize(1000).is_ok());
	for i in 0..150 {
		assert_eq!(map.get(5*i), Some(i));
	}
}

#[test]
fn full_queue() {
	let mut map = HashInt::new(256).unwrap();
	for i in 0..256 {
		assert!(map.insert(5*i, i).is_ok());
	}
	assert_eq!(map.insert(5*256, 256), Err(HashError::Full));
}
