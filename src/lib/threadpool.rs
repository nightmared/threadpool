use std::{thread, io};
use std::marker::PhantomData;
use std::collections::VecDeque;
use lib::messagequeue::*;
use lib::hashint::HashInt;
use lib::thread::*;

// TODO: handle poisoned threads !!!

#[derive(Debug, PartialEq)]
pub enum Command {
    RunTask,
    StopTask,
    Stop
}

#[derive(Debug)]
pub struct Query<T> {
    cmd: Command,
    id: usize,
    val: Option<T>
}

impl<T> Query<T> {
    pub fn run_task(id: usize, val: T) -> Self {
        Query {
            cmd: Command::RunTask,
            id,
            val: Some(val)
        }
    }
    pub fn stop_task(id: usize) -> Self {
        Query {
            cmd: Command::StopTask,
            id,
            val: None
        }
    }
    pub fn stop() -> Self {
        Query {
            cmd: Command::Stop,
            id: 0,
            val: None
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum AnswerState {
    TaskResult,
    Stopped
}

#[derive(Debug)]
pub struct Answer<R> {
    pub state: AnswerState,
    pub id: usize,
    pub val: Option<Result<R, io::Error>>
}

impl<R> Answer<R> {
    fn res(id: usize, val: Option<Result<R, io::Error>>) -> Self {
        Answer {
            state: AnswerState::TaskResult,
            id,
            val
        }
    }
    fn stopped() -> Self {
        Answer {
            state: AnswerState::Stopped,
            id: 0,
            val: None
        }
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
    work_queue: VecDeque<Query<T>>,
    tasks: HashInt<usize>,
    handler_fun: F,
    rx: MessageQueueReader<Query<T>>,
    tx: MessageQueueSender<Answer<R>>
}

impl<T: Send, R: Send, F: Fn(T) -> Result<R, io::Error> + Send> Pool<T, R, F> {
    fn handle_cmd(&mut self, msg: Query<T>) {
        if self.state != PoolState::Running {
            return;
        }
        match msg.cmd {
            Command::RunTask => {
                // Discard tasks with no value
                if msg.val.is_none() {
                    return;
                }
                for i in 0..self.threads.len() {
                    if self.threads[i].is_ready() {
                        self.tasks.insert(msg.id, i);
                        self.threads[i].run(msg.id, msg.val.unwrap());
                        return;
                    }
                }
                self.work_queue.push_front(msg);
            },
            Command::StopTask => unimplemented!(),
            Command::Stop => {
                self.work_queue.clear();
                self.state == PoolState::Stopping;
            }
        }
    }
    fn handle_thread_result(&mut self, thread_idx: usize) {
        // the result of fighting the borrow checker, once again
        {
            let mut thread = &mut self.threads[thread_idx];
            while let Some(x) = thread.rx.read() {
                match x.res {
                    ThreadResult::Stopped => {
                        thread.state = ThreadState::Stopped;
                    },
                    ThreadResult::TaskResult => {
                        self.tasks.remove(x.id);
                        self.tx.send(Answer::res(x.id, x.val));
                        if thread.is_running() {
                            thread.state = ThreadState::Ready;
                        }
                    }
                }
            }
        }
        if self.state == PoolState::Stopping {
            self.threads[thread_idx].stop();
            return;            
        }
        if self.threads[thread_idx].state == ThreadState::Ready {
            self.steal_work(thread_idx);
        }
    }
    fn steal_work(&mut self, thread_id: usize) {
        if self.work_queue.len() > 0 {
            let task = self.work_queue.pop_back().unwrap();
            self.threads[thread_id].run(task.id, task.val.unwrap());
            self.tasks.insert(task.id, thread_id);
        }
    }
    fn run(mut self) {
        loop {
            while let Some(x) = self.rx.read() {
                self.handle_cmd(x);
            }
            for i in 0..self.threads.len() {
                if self.threads[i].rx.is_ready() {
                    self.handle_thread_result(i);
                }
            }
            if self.state == PoolState::Stopping {
                let stopped_threads = self.threads.iter().fold(0, |acc, x| if x.is_stopped() { acc+1 } else { acc });
                if stopped_threads == self.threads.len() {
                    self.tx.send(Answer::stopped());
                    return;
                }
            }
            thread::yield_now();
        }
    }
}

pub struct PoolHandler<T: 'static, R: 'static, F> {
    tx: MessageQueueSender<Query<T>>,
    rx: MessageQueueReader<Answer<R>>,
    phantom: PhantomData<F>
}

impl<T: Send, R: Send, F: Fn(T) -> Result<R, io::Error> + 'static + Clone + Send> PoolHandler<T, R, F> {
    pub fn new(num_workers: usize, cmd_queue_len: usize, workers_queue_len: usize, f: F) -> Result<PoolHandler<T, R, F>, io::Error> {
        let mut threads = Vec::new();
        for i in 0..num_workers {
            threads.push(Thread::new(workers_queue_len, f.clone())?);
        }
        let tasks = HashInt::new(num_workers*2)?;
        let (mut tx1, mut rx1) = MessageQueue(cmd_queue_len)?;
        let (mut tx2, mut rx2) = MessageQueue(cmd_queue_len)?;
        thread::spawn(move || Pool {
            state: PoolState::Running,
            threads,
            work_queue: VecDeque::new(),
            tasks,
            handler_fun: f,
            rx: rx1,
            tx: tx2
        }.run());

        Ok(PoolHandler {
            rx: rx2,
            tx: tx1,
            phantom: PhantomData
        })
    }
    pub fn run(&mut self, id: usize, task: T) -> Result<(), MessageQueueError> {
        self.tx.send(Query::run_task(id, task))
    }
    pub fn blocking_read(&mut self) -> Option<Answer<R>> {
        self.rx.blocking_read()
    }
}

impl<T, R, F> Drop for PoolHandler<T, R, F> {
    fn drop(&mut self) {
        self.tx.send(Query::stop());
    }
}
