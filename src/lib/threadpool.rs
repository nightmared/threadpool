extern crate nix;
use std::{mem, thread, option};
use std::collections::VecDeque;
use nix::sys::epoll;
use nix::unistd;
use lib::messagequeue::*;

#[derive(PartialEq, Debug)]
enum ThreadQueryState {
    Stop,
    RunTask
}

#[derive(Debug)]
struct ThreadQuery<T> {
    state: ThreadQueryState,
    task: Option<T>,
    id: usize
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum ThreadState {
    Ready,
    Running,
    Stopping,
    Stopped,
    Failure
}

#[derive(Debug)]
struct ThreadAnswer<R> {
    state: ThreadState,
    val: Option<Result<R, TPError>>,
    id: usize
}

#[derive(Debug)]
struct TPThread<T, R> {
    internal: thread::JoinHandle<()>,
    state: ThreadState,
    tx: MessageQueueSender<ThreadQuery<T>>,
    rx: MessageQueueReader<ThreadAnswer<R>>
}

impl<T, R> TPThread<T, R> {
    fn stop(&mut self) -> Result<(), TPError> {
        self.tx.send(ThreadQuery::Stop)?;
        self.state = TPThreadState::Stopping;
        Ok(())
    }

    fn run(&mut self, task: T) -> Result<(), TPError> {
        self.tx.send(ThreadQuery::RunTask(task))?;
        self.state = TPThreadState::Running;
        Ok(())
    }
}

#[derive(Debug, PartialEq)]
pub enum CmdQuery<T> {
    Stop,
    StopThread(usize),
    GetInfo,
    AddTask(T)
}

#[derive(Debug, PartialEq)]
pub enum CmdAnswer<R> {
    StoppedThread(usize),
    Stopped,
    TaskSuccess(R),
    TaskFailure(TPError),
    TaskRejected,
    State(TPState),
    ThreadAdded,
    Failed
}

#[derive(Debug, PartialEq)]
pub enum TPError {
    MessageQueueError(MessageQueueError),
    NixError(nix::Error),
    ReadFailed,
    EpollWaitFailed,
    UnwrapingNone
}

impl From<MessageQueueError> for TPError {
    fn from(e: MessageQueueError) -> Self {
        TPError::MessageQueueError(e)
    }
}

impl From<nix::Error> for TPError {
    fn from(e: nix::Error) -> Self {
        TPError::NixError(e)
    }
}

impl From<option::NoneError> for TPError {
    fn from(e: option::NoneError) -> Self {
        TPError::UnwrapingNone
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum TPState {
    Running,
    // Stopping(number of still alive threads)
    Stopping,
    Stopped
}

#[derive(Debug)]
pub struct TP<T, R, F> {
   cmd_rx: MessageQueueReader<CmdQuery<T>>,
   cmd_tx: MessageQueueSender<CmdAnswer<R>>,
   threads: Vec<TPThread<T, R>>,
   ready_threads: usize,
   handler_fun: F,
   state: TPState
}

fn runner_task<T, R, F>(mut rx: MessageQueueReader<ThreadQuery<T>>, mut tx: MessageQueueSender<ThreadAnswer<R>>, worker_fun: F)
    where F: Fn(T) -> Result<R, TPError> {
        while let Some(msg) = rx.blocking_read() {
            match msg {
                ThreadQuery::RunTask(task) => {
                    match worker_fun(task) {
                        Ok(res) => tx.send(ThreadAnswer::TaskResult(res)).unwrap(),
                        Err(e) => tx.send(ThreadAnswer::Error(e)).unwrap()
                    }
                },
                ThreadQuery::Stop => break
            }
        }
        tx.send(ThreadAnswer::Stopped).unwrap();
}

fn create_runner<T, R, F>(f: F) -> Result<TPThread<T, R>, TPError>
    where T: Send + 'static, R: Send + 'static, F: Fn(T) -> Result<R, TPError> + 'static + Clone + Send {
    // TODO: allow to change queues length
    let (tx1, rx1) = MessageQueue(2048)?;
    let (tx2, rx2) = MessageQueue(2048)?;
    let f = f.clone();
    let handle = thread::spawn(|| runner_task(rx1, tx2, f));
    Ok(TPThread {
        internal: handle,
        state: TPThreadState::Ready,
        tx: tx1,
        rx: rx2
    })
}


impl<T: Send + 'static, R: Send + 'static, F: Fn(T) -> Result<R, TPError> + 'static + Clone + Send> TP<T, R, F> {
    pub fn new(rx: MessageQueueReader<CmdQuery<T>>, tx: MessageQueueSender<CmdAnswer<R>>, number_workers: usize, f: F) -> Result<TP<T, R, F>, TPError> {
        let mut threads = Vec::new();
        for _ in 0..number_workers {
            threads.push(create_runner(f.clone())?);
        }
        Ok(TP {
            cmd_rx: rx,
            cmd_tx: tx,
            threads,
            available_threads: number_workers,
            handler_fun: f,
            state: TPState::Running
        })
    }

    fn get_ready_thread(&mut self) -> Option<&mut TPThread<T, R>> {
        if self.available_threads != 0 {
            for i in 0..self.threads.len() {
                if self.threads[i].state == TPThreadState::Ready {
                    return Some(&mut self.threads[i]);
                }
            }
        }
        None
    }

    fn is_ready_thread(&self) -> bool {
        self.available_threads != 0
    }

    fn is_stopped(&self) -> bool {
         for i in 0..self.threads.len() {
            if self.threads[i].state != TPThreadState::Stopped {
                return false;
            }
        }
        true

    }

    fn received_cmd(&mut self) -> Result<(), TPError> {
        while self.cmd_rx.is_ready() {
            let msg = match self.cmd_rx.read() {
                Some(x) => x,
                None => {
                    self.cmd_tx.send(CmdAnswer::Failed)?;
                    return Err(TPError::ReadFailed);
                }
            };
            match msg {
                CmdQuery::GetInfo => self.cmd_tx.send(CmdAnswer::State(self.state.clone()))?,
                CmdQuery::StopThread(n) => self.threads[n].stop()?,
                CmdQuery::Stop => {
                    for i in 0..self.threads.len() {
                        self.threads[i].stop()?;
                    }
                    self.state = TPState::Stopping;
                },
                CmdQuery::AddTask(task) => {
                    if self.state == TPState::Running {
                        // check if there is some thread ready
                        if self.is_ready_thread() {
                            self.get_ready_thread()?.run(task)?;
                        } else {
                            self.work_queue.push_front(task);
                        }
                    } else {
                        self.cmd_tx.send(CmdAnswer::TaskRejected)?;
                    }
                }
            }
        }

        Ok(())
    }

    fn receive_from_thread(&mut self, thread_idx: usize) -> Result<(), TPError> {
        let mut th = &mut self.threads[thread_idx];
        while th.rx.is_ready() {
            let msg = match th.rx.read() {
                Some(x) => x,
                None => {
                    self.cmd_tx.send(CmdAnswer::Failed)?;
                    return Err(TPError::ReadFailed);
                }
            };
            // take care of clearing the work queue when ready (and updating state)
            match msg {
                ThreadAnswer::Error(e) => {
                    self.cmd_tx.send(CmdAnswer::TaskFailure(e))?;
                    if th.state != TPThreadState::Stopping {
                        th.state = TPThreadState::Ready;
                    }
                },
                ThreadAnswer::Failure => {
                    if th.state == TPThreadState::Running || th.state == TPThreadState::Ready {
                        // replace the worker with a new one
                        mem::replace(th, create_runner(self.handler_fun.clone())?);
                    } else {
                        // thread was stopping or stopped, do not touch anything
                        th.state = TPThreadState::Stopped;
                    }
                },
                ThreadAnswer::Stopped => {
                    th.state = TPThreadState::Stopped;
                    // if all threads are stopped, exit
                    if self.is_stopped() {
                        self.state = TPState::Stopped;
                    }
                },
                ThreadAnswer::TaskResult(res) => {
                    self.cmd_tx.send(CmdAnswer::TaskSuccess(res))?;
                    if th.state != TPThreadState::Stopping {
                        th.state = TPThreadState::Ready;
                    }
                }
            }
        }

        Ok(())
    }

    fn tp_loop(mut self) -> Result<(), TPError> {
        while self.state != TPState::Stopped {
            if self.cmd_rx.is_ready() {
                self.received_cmd()?;
            }
            for i in 0..self.threads.len() {
                if let Some(msg) = self.threads[i].rx.read() {
                    self.receive_from_thread(i)?;
                }
            }
            self.run_work_from_queue()?;
            thread::yield_now();
        }
        self.cmd_tx.send(CmdAnswer::Stopped)?;
        Ok(())
    }

    pub fn run(self) -> thread::JoinHandle<Result<(), TPError>> {
       thread::spawn(move || self.tp_loop())
    }
}


pub struct TPHandler<T, R> {
    handle: thread::JoinHandle<Result<(), TPError>>,
    rx: MessageQueueReader<CmdAnswer<R>>,
    tx: MessageQueueSender<CmdQuery<T>>
}

impl<T: Send + 'static, R: Send + 'static> TPHandler<T, R> {
    pub fn new<F: Fn(T) -> Result<R, TPError> + 'static + Clone + Send>(number_workers: usize, f: F) -> Result<TPHandler<T, R>, TPError> {
        let (cmd_tx, mut _cmd_rx) = MessageQueue(1e6 as usize)?;
        let (_cmd_tx, mut cmd_rx) = MessageQueue(1e6 as usize)?;
        let tp = TP::new(_cmd_rx, _cmd_tx, number_workers, f)?;
        let handle = tp.run();
        Ok(TPHandler {
            handle,
            rx: cmd_rx,
            tx: cmd_tx
        })
    }

    pub fn send(&mut self, task: T) -> Result<(), TPError> {
        self.tx.send(CmdQuery::AddTask(task))?;
        Ok(())
    }

    pub fn stop(&mut self, thread: Option<usize>) -> Result<(), TPError> {
        if let Some(t) = thread {
            self.tx.send(CmdQuery::StopThread(t))?;
        } else {
            self.tx.send(CmdQuery::Stop)?;
        }
        Ok(())
    }

    pub fn blocking_read(&mut self) -> Result<CmdAnswer<R>, TPError> {
        let val = self.rx.blocking_read()?;
        Ok(val)
    }
}
