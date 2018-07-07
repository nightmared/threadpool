use lib::messagequeue::*;
use std::{io, thread};
use std::os::unix::thread::JoinHandleExt;
use libc::{pthread_kill, pthread_exit};
use nix::sys::signal::*;

#[derive(Debug)]
pub struct Task<T> {
    pub id: usize,
    pub task: T
}

impl<T> Task<T> {
	#[inline]
    pub fn new(id: usize, task: T) -> Self {
        Task { id, task }
    }
}

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
            match msg {
                ThreadQuery::Stop => {
					self.tx.send(ThreadAnswer::Stopped).unwrap();
					return;
				},
                ThreadQuery::RunTask(Task {id, task }) => self.tx.send(ThreadAnswer::TaskResult { id, val: (self.handler)(task) }).unwrap()
            }
        }
    }
}

#[derive(Debug)]
crate enum ThreadQuery<T> {
    RunTask(Task<T>),
    Stop
}

#[derive(Debug)]
crate enum ThreadAnswer<R> {
    TaskResult { id: usize, val: Result<R, io::Error> },
    Stopped
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
    pub fn run(&mut self, t: Task<T>) -> Result<(), MessageQueueError> {
        self.tx.send(ThreadQuery::RunTask(t))?;
        self.state = ThreadState::Running;
        Ok(())
    }
    pub fn stop(&mut self) -> Result<(), MessageQueueError> {
        self.tx.send(ThreadQuery::Stop)?;
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
