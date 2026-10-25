//! Bounded external work cancelled with its request or server connection.
use crate::rpc::{Error, Result};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::{Notify, Semaphore};

#[derive(Clone)]
pub(crate) struct Tasks {
    slots: Arc<Semaphore>,
    closing: Arc<AtomicBool>,
    active: Arc<AtomicUsize>,
    idle: Arc<Notify>,
}

impl Default for Tasks {
    fn default() -> Self {
        Self {
            slots: Arc::new(Semaphore::new(4)),
            closing: Arc::default(),
            active: Arc::default(),
            idle: Arc::default(),
        }
    }
}

struct Active {
    active: Arc<AtomicUsize>,
    idle: Arc<Notify>,
}
impl Drop for Active {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
        self.idle.notify_waiters();
    }
}

pub(crate) struct Cancellation {
    request: Arc<AtomicBool>,
    closing: Arc<AtomicBool>,
}
impl Cancellation {
    pub fn cancelled(&self) -> bool {
        self.request.load(Ordering::SeqCst) || self.closing.load(Ordering::SeqCst)
    }
}

pub(crate) struct Task<T> {
    handle: tokio::task::JoinHandle<Result<T>>,
    cancel: Arc<AtomicBool>,
}
impl<T> Task<T> {
    pub async fn wait(&mut self) -> Result<T> {
        (&mut self.handle)
            .await
            .map_err(|error| Error::invalid_params(format!("External task failed: {error}")))?
    }
}
impl<T> Drop for Task<T> {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

impl Tasks {
    pub fn start<T: Send + 'static>(
        &self,
        work: impl FnOnce(Cancellation) -> Result<T> + Send + 'static,
    ) -> Result<Task<T>> {
        if self.closing.load(Ordering::SeqCst) {
            return Err(Error::invalid_request());
        }
        let permit = self.slots.clone().try_acquire_owned().map_err(|_| {
            Error::invalid_params("Too many concurrent external requests; retry shortly")
        })?;
        self.active.fetch_add(1, Ordering::SeqCst);
        let active = Active {
            active: self.active.clone(),
            idle: self.idle.clone(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let cancellation = Cancellation {
            request: cancel.clone(),
            closing: self.closing.clone(),
        };
        let handle = tokio::task::spawn_blocking(move || {
            let (_permit, _active) = (permit, active);
            if cancellation.cancelled() {
                return Err(Error::request_cancelled());
            }
            work(cancellation)
        });
        Ok(Task { handle, cancel })
    }

    pub async fn shutdown(&self) {
        self.closing.store(true, Ordering::SeqCst);
        loop {
            let idle = self.idle.notified();
            if self.active.load(Ordering::SeqCst) == 0 {
                break;
            }
            idle.await;
        }
    }
}
