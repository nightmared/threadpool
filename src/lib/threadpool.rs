use std::{mem, thread, option};
use std::collections::VecDeque;
use nix::sys::epoll;
use nix::unistd;
use lib::messagequeue::*;

//struct ThreadState {
//    Ready,
//    Running,
//    Stopping,
//    Stopped
//}
//
//struct ThreadInternal<T, R> {
//    state: ThreadState,
//    rx: MessageQueueSender<ThreadQuery<T>>,
//    tx: MessageQueueReceiver<ThreadAnswer<R>>
//}
//
//struct Thread<T, R> {
//    state: ThreadState,
//    rx: MessageQueueReceiver<ThreadAnswer<R>>,
//    tx: MessageQueueSender<ThreadQuery<T>>
//}
//
//struct ThreadPool<T, R, F> {
//    threads: Vec<Thread<T, R>>,
//    tasks: HashInt<Option<usize>>,
//    handler_fun: F
//}
