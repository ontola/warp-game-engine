//! Bounded, ordered outbound mailboxes. Replace consecutive snapshots before
//! they allocate an unbounded backlog; reliable messages remain barriers.
use std::{collections::VecDeque, sync::{Arc, Mutex}};
use tokio::sync::{Notify, mpsc::error::SendError};

const CAPACITY: usize = 64;
struct Entry<T> { value: T, latest: bool }
struct Inner<T> { queue: VecDeque<Entry<T>>, closed: bool, senders: usize }
struct Shared<T> { inner: Mutex<Inner<T>>, wake: Notify, abort: Notify, latest: fn(&T) -> bool }

/// Host output that never waits for network I/O. A slow peer retains only the newest consecutive
/// snapshot. If reliable barriers fill the bounded mailbox, sending fails and
/// the peer is disconnected rather than silently losing reliable game state.
pub struct HostSender<T> { shared: Arc<Shared<T>> }
pub(crate) struct Receiver<T> { shared: Arc<Shared<T>> }
pub(crate) struct Abort<T> { shared: Arc<Shared<T>> }
impl<T> Abort<T> {
    pub async fn wait(&self) {
        loop {
            let notified = self.shared.abort.notified();
            if self.shared.inner.lock().unwrap().closed { return; }
            notified.await;
        }
    }
}

pub(crate) fn channel<T>(latest: fn(&T) -> bool) -> (HostSender<T>, Receiver<T>) {
    let shared = Arc::new(Shared { inner: Mutex::new(Inner {
        queue: VecDeque::with_capacity(CAPACITY), closed: false, senders: 1,
    }), wake: Notify::new(), abort: Notify::new(), latest });
    (HostSender { shared: shared.clone() }, Receiver { shared })
}

impl<T> Clone for HostSender<T> {
    fn clone(&self) -> Self {
        self.shared.inner.lock().unwrap().senders += 1;
        Self { shared: self.shared.clone() }
    }
}
impl<T> Drop for HostSender<T> {
    fn drop(&mut self) {
        let mut inner = self.shared.inner.lock().unwrap();
        inner.senders -= 1;
        let last = inner.senders == 0;
        drop(inner);
        if last { self.shared.wake.notify_one(); }
    }
}
impl<T> HostSender<T> {
    pub fn send(&self, value: T) -> Result<(), SendError<T>> {
        let latest = (self.shared.latest)(&value);
        let mut inner = self.shared.inner.lock().unwrap();
        if inner.closed { return Err(SendError(value)); }
        let replaced = if latest && inner.queue.back().is_some_and(|e| e.latest) {
            Some(std::mem::replace(&mut inner.queue.back_mut().unwrap().value, value))
        } else {
            if inner.queue.len() == CAPACITY {
                inner.closed = true;
                drop(inner);
                self.shared.wake.notify_one();
                self.shared.abort.notify_one();
                return Err(SendError(value));
            }
            inner.queue.push_back(Entry { value, latest });
            None
        };
        drop(inner);
        // Potentially large obsolete snapshots are reclaimed outside the lock.
        drop(replaced);
        self.shared.wake.notify_one();
        Ok(())
    }
}
impl<T> Receiver<T> {
    pub fn aborted(&self) -> bool { self.shared.inner.lock().unwrap().closed }
    pub fn abort_signal(&self) -> Abort<T> { Abort { shared: self.shared.clone() } }
    pub async fn recv(&mut self) -> Option<T> {
        loop {
            let notified = self.shared.wake.notified();
            {
                let mut inner = self.shared.inner.lock().unwrap();
                if inner.closed { return None; }
                if let Some(entry) = inner.queue.pop_front() { return Some(entry.value); }
                if inner.senders == 0 { return None; }
            }
            notified.await;
        }
    }
}
impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        let queue = {
            let mut inner = self.shared.inner.lock().unwrap();
            inner.closed = true;
            std::mem::take(&mut inner.queue)
        };
        drop(queue);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Debug, PartialEq)]
    enum Msg { Snapshot(u32), Barrier(u32) }
    fn latest(msg: &Msg) -> bool { matches!(msg, Msg::Snapshot(_)) }

    #[tokio::test]
    async fn slow_writer_keeps_only_the_latest_snapshot_between_barriers() {
        let (tx, mut rx) = channel(latest);
        for i in 0..10_000 { tx.send(Msg::Snapshot(i)).unwrap(); }
        assert_eq!(tx.shared.inner.lock().unwrap().queue.len(), 1);
        tx.send(Msg::Barrier(10)).unwrap();
        for i in 10_000..20_000 { tx.send(Msg::Snapshot(i)).unwrap(); }
        tx.send(Msg::Barrier(20)).unwrap();
        tx.send(Msg::Snapshot(20_000)).unwrap();
        drop(tx);
        for expected in [Msg::Snapshot(9_999), Msg::Barrier(10), Msg::Snapshot(19_999), Msg::Barrier(20), Msg::Snapshot(20_000)] {
            assert_eq!(rx.recv().await, Some(expected));
        }
        assert_eq!(rx.recv().await, None);
    }

    #[tokio::test]
    async fn saturated_reliable_queue_disconnects_instead_of_losing_barriers() {
        let (tx, mut rx) = channel(latest);
        for i in 0..CAPACITY { tx.send(Msg::Barrier(i as u32)).unwrap(); }
        assert!(tx.send(Msg::Barrier(CAPACITY as u32)).is_err());
        assert_eq!(tx.shared.inner.lock().unwrap().queue.len(), CAPACITY);
        assert!(rx.recv().await.is_none());
        assert!(tx.send(Msg::Snapshot(1)).is_err());
    }

    #[tokio::test]
    async fn idle_receiver_wakes_for_messages_and_last_sender_drop() {
        let (tx, mut rx) = channel(latest);
        let waiter = tokio::spawn(async move { (rx.recv().await, rx.recv().await) });
        tokio::task::yield_now().await;
        let second = tx.clone();
        drop(tx);
        second.send(Msg::Barrier(7)).unwrap();
        drop(second);
        assert_eq!(tokio::time::timeout(std::time::Duration::from_secs(1), waiter).await.unwrap().unwrap(), (Some(Msg::Barrier(7)), None));
    }

    #[test]
    fn a_dropped_receiver_rejects_later_sends() {
        let (tx, rx) = channel(latest);
        drop(rx);
        assert!(tx.send(Msg::Snapshot(1)).is_err());
    }

    #[tokio::test]
    async fn overflow_cancels_a_writer_blocked_on_network_io() {
        let (tx, mut rx) = channel(latest);
        let abort = rx.abort_signal();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let writer = tokio::spawn(async move {
            rx.recv().await.unwrap();
            started_tx.send(()).unwrap();
            tokio::select! {
                _ = std::future::pending::<()>() => false,
                _ = abort.wait() => true,
            }
        });
        tx.send(Msg::Barrier(0)).unwrap();
        started_rx.await.unwrap();
        for i in 0..CAPACITY { tx.send(Msg::Barrier(i as u32 + 1)).unwrap(); }
        assert!(tx.send(Msg::Barrier(99)).is_err());
        assert!(tokio::time::timeout(std::time::Duration::from_secs(1), writer).await.unwrap().unwrap());
    }
}
