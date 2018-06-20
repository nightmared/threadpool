use test;
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
fn insert_multiple_times() {
	let mut map = HashInt::new(256).unwrap();
	for i in 0..150 {
		assert!(map.insert(5*i, i).is_ok());
	}
	for i in 0..150 {
		assert!(map.insert(5*i, 3*i).is_ok());
	}
	for i in 0..150 {
		assert_eq!(map.get(5*i), Some(3*i));
	}
}

#[test]
fn remove() {
	let mut map = HashInt::new(150).unwrap();
	for i in 0..150 {
		assert!(map.insert(2*i, i).is_ok());
	}
	for i in 0..50 {
		assert_eq!(map.remove(2*i), Ok(()));
	}
	for i in 0..50 {
		assert_eq!(map.get(2*i), None);
	}
	for i in 50..150 {
		assert_eq!(map.get(2*i), Some(i));
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

#[bench]
fn insert_1000(b: &mut test::Bencher) {
	let mut map = HashInt::new(2048).unwrap();
    b.iter(|| for i in 0..1000 {
		map.insert(175*i, i+1).unwrap();
	});
}

#[bench]
fn insert_1000_heavy_load(b: &mut test::Bencher) {
	let mut map = HashInt::new(1050).unwrap();
    b.iter(|| for i in 0..1000 {
		map.insert(175*i, i+1).unwrap();
	});
}
