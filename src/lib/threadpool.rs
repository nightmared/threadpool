use std::{thread, io};
use std::marker::PhantomData;
use std::collections::VecDeque;
use lib::messagequeue::*;
use lib::hashint::{HashInt, HashError};
use lib::thread::*;

// TODO: handle poisoned threads !!!

#[derive(Debug)]
pub enum Query<T> {
    RunTask(Task<T>),
    StopTask { id: usize },
    Stop
}

#[derive(Debug)]
pub enum Answer<R> {
    TaskResult { id: usize, val: Result<R, io::Error> },
    TaskStopped { id: usize },
    Stopped
}

#[derive(Debug, PartialEq)]
pub enum PoolError {
    MessageQueueError,
	HashError,
	UnwrapError
}

impl From<MessageQueueError> for PoolError {
    fn from(_: MessageQueueError) -> Self {
        PoolError::MessageQueueError
    }
}

impl From<HashError> for PoolError {
    fn from(_: HashError) -> Self {
        PoolError::HashError
    }
}

impl From<std::option::NoneError> for PoolError {
    fn from(_: std::option::NoneError) -> Self {
        PoolError::UnwrapError
    }
}

#[derive(Debug, PartialEq)]
enum PoolState {
    Running,
    Stopping
}

struct Pool<T: 'static, R: 'static, F> {
    state: PoolState,
    threads: Vec<Thread<T, R>>,
    work_queue: VecDeque<Task<T>>,
    tasks: HashInt<usize>,
    msg_queue_size: usize,
    handler_fun: F,
    rx: MessageQueueReader<Query<T>>,
    tx: MessageQueueSender<Answer<R>>
}

impl<T: Send, R: Send, F: Fn(T) -> Result<R, io::Error> + Send + 'static + Clone> Pool<T, R, F> {
    fn stop(&mut self) {
        self.work_queue.clear();
        self.state = PoolState::Stopping;
    }
    fn handle_cmd(&mut self, msg: Query<T>) -> Result<(), PoolError> {
        if self.state != PoolState::Running {
            return Ok(());
        }
        match msg {
            Query::RunTask(Task {id, task}) => {
                for i in 0..self.threads.len() {
                    if self.threads[i].is_ready() {
                        self.tasks.insert(id, i)?;
                        self.threads[i].run(Task::new(id, task))?;
                        return Ok(());
                    }
                }
                self.work_queue.push_front(Task { id, task });
				Ok(())
            },
            Query::StopTask{id} => {
                // is task running ?
                if let Some(thread_id) = self.tasks.get(id) {
                    self.threads[thread_id].stop().unwrap();
                    self.tasks.remove(id)?;
                    self.threads[thread_id] = Thread::new(self.msg_queue_size, self.handler_fun.clone()).unwrap();
                    self.tx.send(Answer::TaskStopped{id}).unwrap();
                    return Ok(());
                }
                // Is the task present in the work queue ?
                // Yes, scanning the whole array is really inefficient...
                for pos in 0..self.work_queue.len() {
                    // we only delete the first occurence, so the user must take care of using unique task id
                    if self.work_queue[pos].id == id {
                        self.work_queue.remove(pos);
                        return Ok(());
                    }
                }
                // Task not found, let's ignore it and assume we are done with it
                Ok(())
            },
            Query::Stop => {
                self.stop();
                Ok(())
            }
        }
    }
    fn handle_thread_result(&mut self, thread_idx: usize) -> Result<(), PoolError> {
        // This ugly brace is the result of fighting the borrow checker, once again
        {
            let thread = &mut self.threads[thread_idx];
            while let Some(res) = thread.rx.read() {
                match res {
                    ThreadAnswer::Stopped => {
						thread.state = ThreadState::Stopped;
						return Ok(());
                    },
                    ThreadAnswer::TaskResult { id, val } => {
                        self.tasks.remove(id)?;
                        self.tx.send(Answer::TaskResult{id, val})?;
                        if thread.is_running() {
                            thread.state = ThreadState::Ready;
                        }
                    }
                }
            }
        }
        if self.state == PoolState::Stopping {
            self.threads[thread_idx].stop()?;
            return Ok(());
        }
        if self.threads[thread_idx].state == ThreadState::Ready {
            self.steal_work(thread_idx)?;
        }
		Ok(())
    }
    fn steal_work(&mut self, thread_id: usize) -> Result<(), PoolError> {
        if self.work_queue.len() > 0 {
            let task = self.work_queue.pop_back()?;
            self.tasks.insert(task.id, thread_id)?;
            self.threads[thread_id].run(task)?;
        }
		Ok(())
    }
    fn run(mut self) -> Result<(), PoolError> {
        loop {
            while let Some(x) = self.rx.read() {
                self.handle_cmd(x)?;
            }
            for i in 0..self.threads.len() {
                if self.threads[i].rx.is_ready() {
                    self.handle_thread_result(i)?;
                }
            }
            let stopped_threads = self.threads.iter().fold(0, |acc, x| if x.is_stopped() { acc+1 } else { acc });
            if stopped_threads == self.threads.len() {
                self.tx.send(Answer::Stopped)?;
                return Ok(());
            }
            thread::yield_now();
        }
    }
}

pub struct PoolHandler<T: Send + 'static, R: Send + 'static, F: Fn(T) -> Result<R, io::Error> + 'static + Clone + Send> {
    tx: MessageQueueSender<Query<T>>,
    rx: MessageQueueReader<Answer<R>>,
    phantom: PhantomData<F>
}

impl<T: Send, R: Send, F: Fn(T) -> Result<R, io::Error> + 'static + Clone + Send> PoolHandler<T, R, F> {
    pub fn new(num_workers: usize, cmd_queue_len: usize, workers_queue_len: usize, f: F) -> Result<PoolHandler<T, R, F>, io::Error> {
        let mut threads = Vec::new();
        for _ in 0..num_workers {
            threads.push(Thread::new(workers_queue_len, f.clone())?);
        }
        let tasks = HashInt::new(num_workers*2)?;
        let (tx1, rx1) = message_queue(cmd_queue_len)?;
        let (tx2, rx2) = message_queue(cmd_queue_len)?;
        thread::spawn(move || Pool {
            state: PoolState::Running,
            threads,
            work_queue: VecDeque::new(),
            tasks,
            handler_fun: f,
            msg_queue_size: workers_queue_len,
            rx: rx1,
            tx: tx2
        }.run());

        Ok(PoolHandler {
            rx: rx2,
            tx: tx1,
            phantom: PhantomData
        })
    }
    pub fn run(&mut self, id: usize, task: T) -> Result<(), PoolError> {
        self.tx.send(Query::RunTask(Task::new(id, task)))?;
        Ok(())
    }
    pub fn stop(&mut self) -> Result<(), PoolError> {
        self.tx.send(Query::Stop)?;
        Ok(())
    }
    pub fn stop_task(&mut self, id: usize) -> Result<(), PoolError> {
        self.tx.send(Query::StopTask{id})?;
        Ok(())
    }
    pub fn blocking_read(&mut self) -> Option<Answer<R>> {
        self.rx.blocking_read()
    }
    pub fn read(&mut self) -> Option<Answer<R>> {
        self.rx.read()
    }
}

impl<T: Send, R: Send, F: Fn(T) -> Result<R, io::Error> + 'static + Clone + Send> Drop for PoolHandler<T, R, F> {
    fn drop(&mut self) {
        self.stop();
    }
}
