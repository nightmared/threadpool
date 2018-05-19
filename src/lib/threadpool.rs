extern crate nix;
use std::{io, mem, thread};
use std::collections::VecDeque;
use nix::sys::epoll;
use lib::messagequeue::*;

#[derive(Debug)]
enum ThreadQuery<T> {
    Stop,
    RunTask(T)
}

#[derive(Debug)]
enum ThreadAnswer<R> {
    Stopped,
    TaskResult(R),
    Error(io::Error),
    Failure
}
#[derive(Debug, PartialEq, Clone, Copy)]
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
pub enum CmdQuery<T> {
    Stop,
    StopThread(usize),
    GetInfo,
    AddTask(T)
}

#[derive(Debug)]
pub enum CmdAnswer<R> {
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
pub enum TPError {
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
pub enum TPState {
    // TODO: improve over this very "weak" API
    // Running(total, ready, stopped)
    Running(usize, usize, usize),
    Stopping
}

#[derive(Debug)]
pub struct TP<T, R, F> {
   cmd_rx: MessageQueueReader<CmdQuery<T>>,
   cmd_tx: MessageQueueSender<CmdAnswer<R>>,
   threads: Vec<TPThread<T, R>>,
   // work waiting to be done (no ready threads)
   work_queue: VecDeque<T>,
   handler_fun: F,
   state: TPState
}

fn runner_task<T, R, F>(rx: MessageQueueReader<ThreadQuery<T>>, tx: MessageQueueSender<ThreadAnswer<R>>, worker_fun: F)
    where F: Fn(T) -> Result<R, TPError> {
    tx.send(ThreadAnswer::Stopped).unwrap();
}

fn create_runner<T, R, F>(f: F) -> Result<TPThread<T, R>, TPError>
    where T: Send + 'static, R: Send + 'static, F: Fn(T) -> Result<R, TPError> + 'static + Clone + Send {
    let (tx1, rx1) = MessageQueue(1024)?;
    let (tx2, rx2) = MessageQueue(1024)?;
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
            work_queue: VecDeque::new(),
            handler_fun: f,
            state: TPState::Running(number_workers, number_workers, 0)
        })
    }

    // add one to the number of threads available
    fn ready_inc(&mut self) {
        if let TPState::Running(tot, ready, stopped) = self.state {
            self.state = TPState::Running(tot, ready + 1, stopped);
        }
    }

    // add one to the number of threads stopped
    fn stopped_inc(&mut self) {
        if let TPState::Running(tot, ready, stopped) = self.state {
            self.state = TPState::Running(tot, ready, stopped + 1);
        }
    }

    // Is there at least a thread in the 'Ready' state
    fn is_ready(&self) -> bool {
        if let TPState::Running(tot, ready, stopped) = self.state {
            ready != 0
        } else {
            false
        }
    }


    // To call this function, you have to ensure at least a thread is ready. No checks will be
    // performed !
    fn get_ready_thread(&mut self) -> &mut TPThread<T, R> {
        let mut i = 0;
        while i < self.threads.len() {
            if self.threads[i].state == TPThreadState::Ready {
                break;
            }
            i+=1;
        }
        &mut self.threads[i]
    }

    // At least a thread must be ready to call this function.
    fn run_work_from_queue(&mut self) -> Result<(), TPError> {
        while !self.work_queue.is_empty() && self.is_ready() {
            let task = self.work_queue.pop_back().unwrap();
            let th = self.get_ready_thread();
            th.run(task)?;
        }
        Ok(())
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
                            let mut thread = self.get_ready_thread();
                            thread.run(task)?;
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
            match msg {
                ThreadAnswer::Error(e) => {
                    self.cmd_tx.send(CmdAnswer::TaskFailure(e));
                    th.state = TPThreadState::Ready;
                    self.ready_inc();
                },
                ThreadAnswer::Failure => {
                    let old_state = th.state;
                    if old_state == TPThreadState::Running || old_state == TPThreadState::Ready {
                        // replace the worker with a new one
                        mem::replace(th, create_runner(self.handler_fun.clone())?);
                        // update counters
                        if old_state == TPThreadState::Running {
                            self.ready_inc();
                        }
                    } else {
                        // thread was stopping or stopped, do not touch anything
                        th.state = TPThreadState::Stopped;
                    }
                },
                ThreadAnswer::Stopped => {
                    th.state = TPThreadState::Stopped;
                    self.stopped_inc();
                },
                ThreadAnswer::TaskResult(res) => {
                    th.state = TPThreadState::Ready;
                    self.ready_inc();
                    self.cmd_tx.send(CmdAnswer::TaskSuccess(res))?;
                }
            }
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

    pub fn run(mut self) -> thread::JoinHandle<Result<(), TPError>> {
       thread::spawn(move || self.tp_loop())
    }
}
