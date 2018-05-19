#![feature(crate_visibility_modifier, test)]
extern crate nix;
extern crate libc;

mod lib;
#[cfg(test)] mod tests;

use std::{io, mem, thread};
use std::collections::VecDeque;
use lib::*;
use nix::sys::epoll;

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
#[derive(Debug, PartialEq)]
enum TPThreadState {
    Ready,
    Running,
    Stopping,
    Stopped
}

#[derive(Debug)]
struct TPThread<T, R> {
    internal: thread::JoinHandle<()>,
    state: TPThreadState,
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

#[derive(Debug)]
enum CmdQuery<T> {
    Stop,
    StopThread(usize),
    GetInfo,
    AddTask(T)
}

#[derive(Debug)]
enum CmdAnswer<R> {
    StoppedThread(usize),
    Stopped,
    TaskSuccess(R),
    TaskFailure(io::Error),
    TaskRejected,
    State(TPState),
    ThreadAdded,
    Failed
}

#[derive(Debug, PartialEq)]
enum TPError {
    MessageQueueError(MessageQueueError),
    NixError(nix::Error),
    ReadFailed,
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

#[derive(Debug, PartialEq, Clone)]
enum TPState {
    // TODO: improve over this very "weak" API
    // Running(total, ready, stopped)
    Running(usize, usize, usize),
    Stopping
}

#[derive(Debug)]
struct TP<T, R> {
   cmd_rx: MessageQueueReader<CmdQuery<T>>,
   cmd_tx: MessageQueueSender<CmdAnswer<R>>,
   threads: Vec<TPThread<T, R>>,
   // work waiting to be done (no ready threads)
   work_queue: VecDeque<T>,
   state: TPState
}

fn runner_task<T, R, F>(rx: MessageQueueReader<ThreadQuery<T>>, tx: MessageQueueSender<ThreadAnswer<R>>, worker_fun: F)
    where F: Fn(T) -> Result<R, TPError> {
    tx.send(ThreadAnswer::Started).unwrap();
    tx.send(ThreadAnswer::Stopped).unwrap();
}

impl<T: Send + 'static, R: Send + 'static> TP<T, R> {
    fn new<F>(mut rx: MessageQueueReader<CmdQuery<T>>, tx: MessageQueueSender<CmdAnswer<R>>, number_workers: usize, f: F) -> Result<TP<T, R>, TPError>
        where F: Fn(T) -> Result<R, TPError> + 'static + Clone + Send {
        let mut threads = Vec::new();
        for _ in 0..number_workers {
            let (tx1, rx1) = MessageQueue(2048)?;
            let (tx2, rx2) = MessageQueue(2048)?;
            let f = f.clone();
            let handle = thread::spawn(|| runner_task(rx1, tx2, f));
            let t = TPThread {
                internal: handle,
                state: TPThreadState::Ready,
                tx: tx1,
                rx: rx2
            };
            threads.push(t);
        }
        Ok(TP {
            cmd_rx: rx,
            cmd_tx: tx,
            threads,
            work_queue: VecDeque::new(),
            state: TPState::Running(number_workers, number_workers, 0)
        })
    }

    fn received_cmd(&mut self) -> Result<(), TPError> {
        while self.cmd_rx.is_ready() {
            let msg = match self.cmd_rx.read() {
                Ok(x) => x,
                Err(_) => {
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
                    if let TPState::Running(_, ready, _) = self.state {
                        // check if there is some thread ready
                        if ready == 0 {
                            self.work_queue.push_front(task);
                        } else {
                            // scan for a ready thread
                            let mut thread = {
                                let mut i = 0;
                                while i < self.threads.len() {
                                    if self.threads[i].state == TPThreadState::Ready {
                                        break;
                                    }
                                    i+=1;
                                }
                                &mut self.threads[i]
                            };
                            thread.run(task);
                        }
                    } else {
                        self.cmd_tx.send(CmdAnswer::TaskRejected)?;
                    }
                }
            }
        }

        Ok(())
    }

    fn receive_from_thread(&mut self, th: &mut TPThread<T, R>) -> Result<(), TPError> {
        while th.rx.is_ready() {
            let msg = match th.rx.read() {
                Ok(x) => x,
                Err(_) => {
                    self.cmd_tx.send(CmdAnswer::Failed)?;
                    return Err(TPError::ReadFailed);
                }
            };
            // take care of clearing the work queue when ready (and updating state)
            // TODO: everything
        }

        Ok(())
    }

    fn tp_loop(mut self) -> Result<(), TPError> {
        let epfd = epoll::epoll_create1(epoll::EpollCreateFlags::EPOLL_CLOEXEC)?;
        // TODO: handle peer disconnection (and threads panicking)
        let mut ev = epoll::EpollEvent::new(epoll::EpollFlags::EPOLLIN, 0);
        epoll::epoll_ctl(epfd, epoll::EpollOp::EpollCtlAdd, self.cmd_rx.get_fd(), Some(&mut ev));
        for i in 0..self.threads.len() {
            // Behold ! Dark magic is happening here ! (Yes, it's only dark because it's not arcane enought
            // to be called black magic). We store a pointer to the queue in the data, which allow us to call
            // thread.rx.read() directly on the data returned by epoll. (I wonder what will hapen with 128
            // bit architectures when thoses will exist).
            // Besides, the whole point of this is that adding/deleting threads shouldn't impact epoll in
            // any way, thus referencing threads by their index in self.threads is not workable.
            let mut ev = epoll::EpollEvent::new(epoll::EpollFlags::EPOLLIN, &mut self.threads[i] as *mut TPThread<T, R> as u64);
            epoll::epoll_ctl(epfd, epoll::EpollOp::EpollCtlAdd, self.threads[i].rx.get_fd(), Some(&mut ev))?;
        }

        // A listener per worker thread + the command queue
        let max_events = self.threads.len()+1;
        let mut events_vec = Vec::with_capacity(max_events);
        for _ in 0..max_events {
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
            // Nothing interesting here, go on (probably some spurious interrupt)
            if res == 0 { continue; }

            for i in 0..res {
                let reader_ptr = events_vec[i].data();
                if reader_ptr == 0 {
                    // Command query (aka. command received on self.cmd_rx)
                    self.received_cmd()?;
                } else {
                    // Because who doesn't like transmut'ing stuff ?
                    let mut reader: &mut TPThread<T, R> = unsafe { mem::transmute(reader_ptr) };
                    self.receive_from_thread(&mut reader)?;
                }
            }
        }
    }

    fn run(mut self) -> thread::JoinHandle<Result<(), TPError>> {
       thread::spawn(move || self.tp_loop())
    }
}

fn handler(x: usize) -> Result<usize, TPError> {
    Ok(x+1)
}

fn main() -> Result<(), TPError> {
    let (cmd_tx, mut _cmd_rx) = MessageQueue(25)?;
    let (_cmd_tx, mut cmd_rx) = MessageQueue(25)?;
    let tp = TP::new(_cmd_rx, _cmd_tx, 2, handler)?;
    cmd_tx.send(CmdQuery::Stop)?;
    let joinhandle = tp.run();
    loop {
        let msg = cmd_rx.blocking_read().unwrap();
        println!("{:?}", msg);
    }
    println!("{:?}", joinhandle.join());
    //tp.add_task(15);
    Ok(())
}
