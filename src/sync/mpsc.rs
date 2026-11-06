//! Multi-producer, single-consumer FIFO queue communication primitives.
//!
//! A `Mutex<VecDeque>` plus two condition variables: simple rather than
//! lock-free.

use alloc_crate::collections::VecDeque;
use alloc_crate::sync::Arc;
use core::error;
use core::fmt;
use core::time::Duration;

use super::{Condvar, Mutex, MutexGuard};
use crate::time::Instant;

struct State<T> {
    queue: VecDeque<T>,
    senders: usize,
    receiver_alive: bool,
    /// receivers blocked in recv: a zero-capacity channel hands values only
    /// to a waiting receiver
    recv_waiting: usize,
}

struct Chan<T> {
    state: Mutex<State<T>>,
    not_empty: Condvar,
    not_full: Condvar,
    bound: Option<usize>,
}

impl<T> Chan<T> {
    fn lock(&self) -> MutexGuard<'_, State<T>> {
        self.state.lock().unwrap()
    }
}

pub struct Sender<T> {
    chan: Arc<Chan<T>>,
}

pub struct SyncSender<T> {
    chan: Arc<Chan<T>>,
}

pub struct Receiver<T> {
    chan: Arc<Chan<T>>,
}

unsafe impl<T: Send> Send for Sender<T> {}
unsafe impl<T: Send> Sync for Sender<T> {}
unsafe impl<T: Send> Send for SyncSender<T> {}
unsafe impl<T: Send> Sync for SyncSender<T> {}
unsafe impl<T: Send> Send for Receiver<T> {}

#[derive(PartialEq, Eq, Clone, Copy)]
pub struct SendError<T>(pub T);

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub struct RecvError;

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum TryRecvError {
    Empty,
    Disconnected,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum RecvTimeoutError {
    Timeout,
    Disconnected,
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum TrySendError<T> {
    Full(T),
    Disconnected(T),
}

fn new_chan<T>(bound: Option<usize>) -> Arc<Chan<T>> {
    Arc::new(Chan {
        state: Mutex::new(State { queue: VecDeque::new(), senders: 1, receiver_alive: true, recv_waiting: 0 }),
        not_empty: Condvar::new(),
        not_full: Condvar::new(),
        bound,
    })
}

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let chan = new_chan(None);
    (Sender { chan: chan.clone() }, Receiver { chan })
}

/// A bounded channel. With a bound of 0 every send waits for a receiver
/// (rendezvous).
pub fn sync_channel<T>(bound: usize) -> (SyncSender<T>, Receiver<T>) {
    let chan = new_chan(Some(bound));
    (SyncSender { chan: chan.clone() }, Receiver { chan })
}

impl<T> Sender<T> {
    pub fn send(&self, t: T) -> Result<(), SendError<T>> {
        let mut st = self.chan.lock();
        if !st.receiver_alive {
            return Err(SendError(t));
        }
        st.queue.push_back(t);
        drop(st);
        self.chan.not_empty.notify_one();
        Ok(())
    }
}

impl<T> SyncSender<T> {
    fn has_room(&self, st: &State<T>) -> bool {
        match self.chan.bound.unwrap() {
            0 => st.recv_waiting > st.queue.len(),
            bound => st.queue.len() < bound,
        }
    }

    pub fn send(&self, t: T) -> Result<(), SendError<T>> {
        let mut st = self.chan.lock();
        loop {
            if !st.receiver_alive {
                return Err(SendError(t));
            }
            if self.has_room(&st) {
                break;
            }
            st = self.chan.not_full.wait(st).unwrap();
        }
        st.queue.push_back(t);
        drop(st);
        self.chan.not_empty.notify_one();
        Ok(())
    }

    pub fn try_send(&self, t: T) -> Result<(), TrySendError<T>> {
        let mut st = self.chan.lock();
        if !st.receiver_alive {
            return Err(TrySendError::Disconnected(t));
        }
        if !self.has_room(&st) {
            return Err(TrySendError::Full(t));
        }
        st.queue.push_back(t);
        drop(st);
        self.chan.not_empty.notify_one();
        Ok(())
    }
}

macro_rules! sender_common {
    ($name:ident) => {
        impl<T> Clone for $name<T> {
            fn clone(&self) -> Self {
                self.chan.lock().senders += 1;
                $name { chan: self.chan.clone() }
            }
        }

        impl<T> Drop for $name<T> {
            fn drop(&mut self) {
                let mut st = self.chan.lock();
                st.senders -= 1;
                let last = st.senders == 0;
                drop(st);
                if last {
                    self.chan.not_empty.notify_all();
                }
            }
        }

        impl<T> fmt::Debug for $name<T> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($name)).finish_non_exhaustive()
            }
        }
    };
}

sender_common!(Sender);
sender_common!(SyncSender);

impl<T> Receiver<T> {
    fn take(&self, st: &mut State<T>) -> Option<T> {
        let v = st.queue.pop_front();
        if v.is_some() && self.chan.bound.is_some() {
            self.chan.not_full.notify_one();
        }
        v
    }

    /// Block until a value may be available, letting zero-capacity senders
    /// know a receiver is waiting.
    fn wait<'a>(&self, mut st: MutexGuard<'a, State<T>>, timeout: Option<Duration>) -> (MutexGuard<'a, State<T>>, bool) {
        st.recv_waiting += 1;
        if self.chan.bound == Some(0) {
            self.chan.not_full.notify_one();
        }
        let (mut st, timed_out) = match timeout {
            None => (self.chan.not_empty.wait(st).unwrap(), false),
            Some(d) => {
                let (st, r) = self.chan.not_empty.wait_timeout(st, d).unwrap();
                (st, r.timed_out())
            }
        };
        st.recv_waiting -= 1;
        (st, timed_out)
    }

    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        let mut st = self.chan.lock();
        match self.take(&mut st) {
            Some(v) => Ok(v),
            None if st.senders == 0 => Err(TryRecvError::Disconnected),
            None => Err(TryRecvError::Empty),
        }
    }

    pub fn recv(&self) -> Result<T, RecvError> {
        let mut st = self.chan.lock();
        loop {
            if let Some(v) = self.take(&mut st) {
                return Ok(v);
            }
            if st.senders == 0 {
                return Err(RecvError);
            }
            st = self.wait(st, None).0;
        }
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<T, RecvTimeoutError> {
        match Instant::now().checked_add(timeout) {
            Some(deadline) => self.recv_deadline(deadline),
            None => self.recv().map_err(|_| RecvTimeoutError::Disconnected),
        }
    }

    pub fn recv_deadline(&self, deadline: Instant) -> Result<T, RecvTimeoutError> {
        let mut st = self.chan.lock();
        loop {
            if let Some(v) = self.take(&mut st) {
                return Ok(v);
            }
            if st.senders == 0 {
                return Err(RecvTimeoutError::Disconnected);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(RecvTimeoutError::Timeout);
            }
            st = self.wait(st, Some(deadline - now)).0;
        }
    }

    pub fn iter(&self) -> Iter<'_, T> {
        Iter { rx: self }
    }

    pub fn try_iter(&self) -> TryIter<'_, T> {
        TryIter { rx: self }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        let mut st = self.chan.lock();
        st.receiver_alive = false;
        st.queue.clear();
        drop(st);
        self.chan.not_full.notify_all();
    }
}

impl<T> fmt::Debug for Receiver<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Receiver").finish_non_exhaustive()
    }
}

pub struct Iter<'a, T: 'a> {
    rx: &'a Receiver<T>,
}

pub struct TryIter<'a, T: 'a> {
    rx: &'a Receiver<T>,
}

pub struct IntoIter<T> {
    rx: Receiver<T>,
}

impl<T> Iterator for Iter<'_, T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.rx.recv().ok()
    }
}

impl<T> Iterator for TryIter<'_, T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.rx.try_recv().ok()
    }
}

impl<T> Iterator for IntoIter<T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.rx.recv().ok()
    }
}

impl<'a, T> IntoIterator for &'a Receiver<T> {
    type Item = T;
    type IntoIter = Iter<'a, T>;
    fn into_iter(self) -> Iter<'a, T> {
        self.iter()
    }
}

impl<T> IntoIterator for Receiver<T> {
    type Item = T;
    type IntoIter = IntoIter<T>;
    fn into_iter(self) -> IntoIter<T> {
        IntoIter { rx: self }
    }
}

impl<T> fmt::Debug for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SendError").finish_non_exhaustive()
    }
}

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("sending on a closed channel")
    }
}

impl<T> error::Error for SendError<T> {}

impl<T> fmt::Debug for TrySendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TrySendError::Full(..) => f.write_str("Full(..)"),
            TrySendError::Disconnected(..) => f.write_str("Disconnected(..)"),
        }
    }
}

impl<T> fmt::Display for TrySendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TrySendError::Full(..) => f.write_str("sending on a full channel"),
            TrySendError::Disconnected(..) => f.write_str("sending on a closed channel"),
        }
    }
}

impl<T> error::Error for TrySendError<T> {}

impl<T> From<SendError<T>> for TrySendError<T> {
    fn from(err: SendError<T>) -> TrySendError<T> {
        TrySendError::Disconnected(err.0)
    }
}

impl fmt::Display for RecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("receiving on a closed channel")
    }
}

impl error::Error for RecvError {}

impl fmt::Display for TryRecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TryRecvError::Empty => f.write_str("receiving on an empty channel"),
            TryRecvError::Disconnected => f.write_str("receiving on a closed channel"),
        }
    }
}

impl error::Error for TryRecvError {}

impl fmt::Display for RecvTimeoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecvTimeoutError::Timeout => f.write_str("timed out waiting on channel"),
            RecvTimeoutError::Disconnected => f.write_str("channel is empty and sending half is closed"),
        }
    }
}

impl error::Error for RecvTimeoutError {}

impl From<RecvError> for TryRecvError {
    fn from(_: RecvError) -> TryRecvError {
        TryRecvError::Disconnected
    }
}

impl From<RecvError> for RecvTimeoutError {
    fn from(_: RecvError) -> RecvTimeoutError {
        RecvTimeoutError::Disconnected
    }
}
