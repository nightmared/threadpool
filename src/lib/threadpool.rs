use std::{mem, thread, io};
use lib::messagequeue::*;
use lib::hashint::HashInt;
use lib::thread::*;

enum ThreadPoolState {
    Running,
    Stopping
}

struct ThreadPool<T, R, F> {
    threads: Vec<Thread<T, R>>,
    tasks: HashInt<usize>,
    handler_fun: F
}

impl<T: Send + 'static, R: Send + 'static, F> ThreadPool<T, R, F> where F: Fn(&T) -> Result<R, io::Error> {
    fn new(num_workers: usize, cmd_queue_len: usize, workers_queue_len: usize, f: F) -> Result<ThreadPool<T, R, F>, io::Error> {
		let mut threads = Vec::new();
		for i in 0..num_workers {
			match Thread::new(workers_queue_len) {
				Ok(x) => threads.push(x),
				Err(_) => return Err(io::Error::new(io::ErrorKind::Other, "MessageQueueError"))
			}
		}
		let tasks = match HashInt::new(num_workers*2) {
			Ok(x) => x,
			Err(_) => return Err(io::Error::new(io::ErrorKind::Other, "HashIntError"))
		};
        Ok(ThreadPool {
			threads,
			tasks,
			handler_fun: f
		})
    }
}
