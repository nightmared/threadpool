use lib::messagequeue::*;
use std::{io, thread};

crate enum ThreadState {
    Ready,
    Running,
    Stopping,
    Stopped
}

crate struct ThreadInternal<T, R> {
    rx: MessageQueueReader<ThreadQuery<T>>,
    tx: MessageQueueSender<ThreadAnswer<R>>
}

impl<T, R> ThreadInternal<T, R> {
    pub fn handle_msg(&mut self, msg: ThreadQuery<T>) {

    }

    pub fn run(mut self) {
        loop {
            let msg = match self.rx.blocking_read() {
                Some(x) => x,
                None => {
                    self.tx.send(ThreadAnswer::fail()).unwrap();
                    return;
                }
            };
            self.handle_msg(msg);
        }
    }
}

crate enum ThreadOperation {
    Stop,
    RunTask
}

crate struct ThreadQuery<T> {
    val: Option<T>,
    op: ThreadOperation
}

impl<T> ThreadQuery<T> {
    pub fn stop() -> Self {
        ThreadQuery {
            val: None,
            op: ThreadOperation::Stop
        }
    }
    pub fn run(task: T) -> Self {
        ThreadQuery {
            val: Some(task),
            op: ThreadOperation::RunTask
        }
    }
}

crate enum ThreadResult {
    Failure,
    TaskResult
}

crate struct ThreadAnswer<R> {
    val: Option<Result<R, io::Error>>,
    res: ThreadResult
}


impl<R> ThreadAnswer<R> {
    pub fn fail() -> Self {
        ThreadAnswer {
            val: None,
            res: ThreadResult::Failure
        }
    }
    pub fn res(task: Result<R, io::Error>) -> Self {
        ThreadAnswer {
            val: Some(task),
            res: ThreadResult::TaskResult
        }
    }
}

crate struct Thread<T, R> {
    state: ThreadState,
    rx: MessageQueueReader<ThreadAnswer<R>>,
    tx: MessageQueueSender<ThreadQuery<T>>
}

impl<T: Send + 'static, R: Send + 'static> Thread<T, R> {
    pub fn new(message_queue_size: usize) -> Result<Thread<T, R>, MessageQueueError> {
        let (mut tx1, mut rx1) = MessageQueue(message_queue_size)?;
        let (mut tx2, mut rx2) = MessageQueue(message_queue_size)?;
		let th = thread::spawn(move || ThreadInternal {
			rx: rx1,
			tx: tx2
		}.run());
		Ok(Thread {
			state: ThreadState::Running,
			rx: rx2,
			tx: tx1
		})
	}
}
