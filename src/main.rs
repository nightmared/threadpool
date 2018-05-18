#![feature(crate_visibility_modifier)]
extern crate nix;
extern crate libc;

use std::thread;
use std::io;
use lib::*;
use nix::sys::epoll;

mod lib;
#[cfg(test)]
mod tests;

#[derive(Debug)]
enum ThreadQuery<T> {
    Stop,
    RunTask(T)
}

#[derive(Debug)]
enum ThreadAnswer<R> {
    Started,
    Stopped,
    TaskResult(R),
    Error(io::Error)
}

#[derive(Debug)]
enum TPThreadState {
    Starting,
    Running,
    Ready,
    Stopping
}

#[derive(Debug)]
struct TPThread<T, R> {
    internal: thread::JoinHandle<()>,
    state: TPThreadState,
    tx: MessageQueueSender<ThreadQuery<T>>,
    rx: MessageQueueReader<ThreadAnswer<R>>
}

#[derive(Debug)]
enum CmdQuery<T> {
    Stop,
    StopThread(usize),
    GetNumberThreads,
    GetNumberRunningThreads,
    AddThread(Option<usize>),
    AddTask(T),
}

#[derive(Debug)]
enum CmdAnswer<R> {
    StoppedThread(usize),
    Stopped,
    TaskSucces(R),
    TaskFailure(io::Error),
    NumberThreads(usize),
    NumberRunningThread(usize),
    ThreadAdded,
    Failed
}

#[derive(Debug, PartialEq)]
enum TPError {
    MessageQueueError(MessageQueueError),
    NixError(nix::Error),
    EpollWaitFailed
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

#[derive(Debug)]
struct TP<T, R> {
   cmd_rx: MessageQueueReader<CmdQuery<T>>,
   cmd_tx: MessageQueueSender<CmdAnswer<R>>,
   threads: Vec<TPThread<T, R>>
}

fn runner_task<T, R, F>(rx: MessageQueueReader<ThreadQuery<T>>, tx: MessageQueueSender<ThreadAnswer<R>>, worker_fun: F)
    where F: Fn(T) -> Result<R, TPError> {
    tx.send(ThreadAnswer::Started).unwrap();
    tx.send(ThreadAnswer::Stopped).unwrap();
}

impl<T: Send + 'static, R: Send + 'static> TP<T, R> {
    fn new<F>(rx: MessageQueueReader<CmdQuery<T>>, tx: MessageQueueSender<CmdAnswer<R>>, number_workers: usize, f: F) -> Result<TP<T, R>, TPError>
        where F: Fn(T) -> Result<R, TPError> + 'static + Clone + Send {
        let mut threads = Vec::new();
        for _ in 0..number_workers {
            let (tx1, rx1) = MessageQueue(2048)?;
            let (tx2, rx2) = MessageQueue(2048)?;
            let f = f.clone();
            let handle = thread::spawn(|| runner_task(rx1, tx2, f));
            let t = TPThread {
                internal: handle,
                state: TPThreadState::Starting,
                tx: tx1,
                rx: rx2
            };
            threads.push(t);
        }
        Ok(TP {
            cmd_rx: rx,
            cmd_tx: tx,
            threads
        })
    }

    fn tp_loop(self) -> Result<(), TPError> {
        let epfd = epoll::epoll_create1(epoll::EpollCreateFlags::EPOLL_CLOEXEC)?;
        // TODO: handle peer disconnection (and threads panicking)
        let mut ev = epoll::EpollEvent::new(epoll::EpollFlags::EPOLLIN, 0);
        epoll::epoll_ctl(epfd, epoll::EpollOp::EpollCtlAdd, self.cmd_rx.get_fd(), Some(&mut ev));
        println!("{}", nix::errno::errno());
        for i in 0..self.threads.len() {
            epoll::epoll_ctl(epfd, epoll::EpollOp::EpollCtlAdd, self.threads[i].rx.get_fd(), Some(&mut ev))?;
        }

        // A listener per worker thread + the command queue
        let max_events = self.threads.len()+1;
        let mut events_vec = Vec::with_capacity(max_events);
        for _ in 0..events_vec.len() {
            events_vec.push(epoll::EpollEvent::empty());
        }
        loop {
            let res = match epoll::epoll_wait(epfd, &mut events_vec, -1) {
                Ok(x) => x,
                Err(_) => {
                    self.cmd_tx.send(CmdAnswer::Failed)?;
                    return Err(TPError::EpollWaitFailed);
                }
            };
            if res == 0 {
                continue;
            }
            println!("{}", res);
        }
    }

    fn run(self) -> thread::JoinHandle<Result<(), TPError>> {
       thread::spawn(move || self.tp_loop())
    }
}

fn handler(x: usize) -> Result<usize, TPError> {
    Ok(x+1)
}

fn main() -> Result<(), TPError> {
    println!("Hello, world!");
    let (cmd_tx, _cmd_rx) = MessageQueue(25)?;
    let (_cmd_tx, cmd_rx) = MessageQueue(25)?;
    let tp = TP::new(_cmd_rx, _cmd_tx, 2, handler)?;
    println!("{:?}", tp.run().join());
    cmd_tx.send(CmdQuery::Stop)?;
    //tp.add_task(15);
    Ok(())
}
