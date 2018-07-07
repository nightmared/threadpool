use lib::messagequeue::*;
use std::{io, thread};
use std::os::unix::thread::JoinHandleExt;
use libc::{pthread_kill, pthread_exit};
use nix::sys::signal::*;

#[derive(Debug, PartialEq)]
crate enum ThreadState {
    Ready,
    Running,
    Stopping,
    Stopped
}

struct ThreadInternal<T, R, F> {
    rx: MessageQueueReader<ThreadQuery<T>>,
    tx: MessageQueueSender<ThreadAnswer<R>>,
    handler: F
}

impl<T, R, F: Fn(T) -> Result<R, io::Error>> ThreadInternal<T, R, F> {
    pub fn run(mut self) {
        loop {
            let msg = self.rx.blocking_read().unwrap();
            match msg.op {
                ThreadOperation::Stop => {
					self.tx.send(ThreadAnswer::stopped()).unwrap();
					return;
				},
                ThreadOperation::RunTask => self.tx.send(ThreadAnswer::res(msg.id, (self.handler)(msg.val.unwrap()))).unwrap()
            }
        }
    }
}

#[derive(Debug, PartialEq)]
crate enum ThreadOperation {
    Stop,
    RunTask
}

crate struct ThreadQuery<T> {
    pub val: Option<T>,
    pub id: usize,
    pub op: ThreadOperation
}

impl<T> ThreadQuery<T> {
    pub fn stop() -> Self {
        ThreadQuery {
            val: None,
            id: 0,
            op: ThreadOperation::Stop
        }
    }
    pub fn run(id: usize, task: T) -> Self {
        ThreadQuery {
            val: Some(task),
            id,
            op: ThreadOperation::RunTask
        }
    }
}

#[derive(Debug, PartialEq)]
crate enum ThreadResult {
    TaskResult,
    Stopped
}

crate struct ThreadAnswer<R> {
    pub res: ThreadResult,
    pub id: usize,
    pub val: Option<Result<R, io::Error>>
}


impl<R> ThreadAnswer<R> {
    pub fn res(id: usize, task: Result<R, io::Error>) -> Self {
        ThreadAnswer {
            res: ThreadResult::TaskResult,
            id,
            val: Some(task)
        }
    }
    pub fn stopped() -> Self {
        ThreadAnswer {
            res: ThreadResult::Stopped,
            id: 0,
            val: None
        }
    }
}

crate struct Thread<T, R> {
    crate state: ThreadState,
    crate rx: MessageQueueReader<ThreadAnswer<R>>,
    crate tx: MessageQueueSender<ThreadQuery<T>>,
    crate handle: thread::JoinHandle<()>
}

impl<T: Send + 'static, R: Send + 'static> Thread<T, R> {
    pub fn new<F>(message_queue_size: usize, f: F) -> Result<Thread<T, R>, MessageQueueError> 
        where F: Fn(T) -> Result<R, io::Error> + 'static + Send {
        let (tx1, rx1) = message_queue(message_queue_size)?;
        let (tx2, rx2) = message_queue(message_queue_size)?;
        let th = thread::spawn(move || ThreadInternal {
            rx: rx1,
            tx: tx2,
            handler: f
        }.run());
        Ok(Thread {
            state: ThreadState::Ready,
            rx: rx2,
            tx: tx1,
            handle: th
        })
    }
    pub fn run(&mut self, id: usize, task: T) -> Result<(), MessageQueueError> {
        self.tx.send(ThreadQuery::run(id, task))?;
        self.state = ThreadState::Running;
        Ok(())
    }
    pub fn stop(&mut self) -> Result<(), MessageQueueError> {
        self.tx.send(ThreadQuery::stop())?;
        self.state = ThreadState::Stopping;
        Ok(())
    }
    pub fn is_ready(&self) -> bool {
        self.state == ThreadState::Ready
    }
    pub fn is_stopped(&self) -> bool {
        self.state == ThreadState::Stopped
    }
    pub fn is_running(&self) -> bool {
        self.state == ThreadState::Running
    }
}
