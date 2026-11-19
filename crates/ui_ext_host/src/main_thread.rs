//! Jobs for the main thread from extension threads, for the 0.4 hosts whose calls are
//! synchronous (`ui_ext::context::ContextHost`, `ui_ext::review::ReviewHost`): a job runs in a
//! main-thread loop and answers on a channel the caller waits on, up to a deadline.
//!
//! Never wait from the main thread: the loop could not run, and the call would time out. The
//! hosts call their sinks from background tasks for that reason (a sink may call back into a
//! host, as the runtime's privacy filter does with `is_private`).

use std::{sync::mpsc as std_mpsc, time::Duration};

use futures::{StreamExt as _, channel::mpsc};
use gpui::{App, Global};

type Job = Box<dyn FnOnce(&mut App) + Send>;

/// The loop's inbox. Cloned into every host.
#[derive(Clone)]
pub(crate) struct MainThread(mpsc::UnboundedSender<Job>);

impl Global for MainThread {}

/// Where a job answers. Sending fails once the caller stopped waiting: a job that would leave
/// something open (a review) closes it again then.
pub(crate) struct Reply<T>(std_mpsc::SyncSender<T>);

impl<T> Reply<T> {
    /// Whether the caller still waits and got `value`.
    pub(crate) fn send(self, value: T) -> bool {
        self.0.send(value).is_ok()
    }
}

impl MainThread {
    pub(crate) fn global(cx: &mut App) -> Self {
        if let Some(main) = cx.try_global::<Self>() {
            return main.clone();
        }
        let (sender, mut jobs) = mpsc::unbounded::<Job>();
        cx.spawn(async move |cx| {
            while let Some(job) = jobs.next().await {
                cx.update(job);
            }
        })
        .detach();
        let main = Self(sender);
        cx.set_global(main.clone());
        main
    }

    /// Runs `job` on the main thread without waiting for it. `false` once the app is gone.
    pub(crate) fn post(&self, job: impl FnOnce(&mut App) + Send + 'static) -> bool {
        self.0.unbounded_send(Box::new(job)).is_ok()
    }

    /// Runs `job` on the main thread and waits up to `timeout` for its answer. `None` when the
    /// app is gone, the job dropped its reply, or the deadline passed. Not from the main thread.
    pub(crate) fn call<T: Send + 'static>(
        &self,
        timeout: Duration,
        job: impl FnOnce(&mut App, Reply<T>) + Send + 'static,
    ) -> Option<T> {
        let (sender, receiver) = std_mpsc::sync_channel(1);
        let reply = Reply(sender);
        if !self.post(move |cx| job(cx, reply)) {
            return None;
        }
        receiver.recv_timeout(timeout).ok()
    }
}
