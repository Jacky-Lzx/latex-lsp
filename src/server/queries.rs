//! Analysis workers retain capacity until computation actually finishes.
use crate::rpc::{Error, Result};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::{Notify, Semaphore};

#[derive(Clone)]
pub(crate) struct Queries {
    slots: Arc<Semaphore>,
    closing: Arc<AtomicBool>,
    active: Arc<AtomicUsize>,
    idle: Arc<Notify>,
}

pub(crate) struct Cancellation {
    request: Arc<AtomicBool>,
    closing: Arc<AtomicBool>,
}

impl Cancellation {
    pub(crate) fn check(&self) -> Result<()> {
        if self.request.load(Ordering::Relaxed) || self.closing.load(Ordering::Relaxed) {
            Err(Error::request_cancelled())
        } else {
            Ok(())
        }
    }
}

struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

struct Active(Queries);

impl Drop for Active {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
        self.0.idle.notify_waiters();
    }
}

impl Default for Queries {
    fn default() -> Self {
        Self::new(4)
    }
}

impl Queries {
    fn new(limit: usize) -> Self {
        Self {
            slots: Arc::new(Semaphore::new(limit)),
            closing: Arc::default(),
            active: Arc::default(),
            idle: Arc::default(),
        }
    }

    pub(crate) async fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce(&Cancellation) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let permit = self
            .slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| Error::request_cancelled())?;
        let request = Arc::new(AtomicBool::new(false));
        let _cancel = CancelOnDrop(request.clone());
        let cancellation = Cancellation {
            request,
            closing: self.closing.clone(),
        };
        cancellation.check()?;
        self.active.fetch_add(1, Ordering::SeqCst);
        let active = Active(self.clone());
        tokio::task::spawn_blocking(move || {
            let (_permit, _active) = (permit, active);
            cancellation.check()?;
            let result = work(&cancellation)?;
            cancellation.check()?;
            Ok(result)
        })
        .await
        .map_err(|_| Error::internal_error())?
    }

    pub(crate) async fn shutdown(&self) {
        self.closing.store(true, Ordering::Relaxed);
        self.slots.close();
        loop {
            let idle = self.idle.notified();
            if self.active.load(Ordering::SeqCst) == 0 {
                break;
            }
            idle.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn cancelled_workers_keep_capacity_and_shutdown_signals_running_work() {
        let queries = Queries::new(1);
        let (started, start) = oneshot::channel();
        let (finish, finished) = oneshot::channel();
        let first_queries = queries.clone();
        let first = tokio::spawn(async move {
            first_queries
                .run(move |cancel| {
                    let _ = started.send(());
                    let _ = finished.blocking_recv();
                    cancel.check()
                })
                .await
        });
        start.await.unwrap();
        first.abort();
        assert!(first.await.unwrap_err().is_cancelled());
        assert_eq!(queries.slots.available_permits(), 0);
        let waiting_queries = queries.clone();
        let waiting = tokio::spawn(async move {
            waiting_queries
                .run::<()>(|_| panic!("queued work ran after shutdown"))
                .await
        });
        let shutting_down = tokio::spawn({
            let queries = queries.clone();
            async move { queries.shutdown().await }
        });
        tokio::task::yield_now().await;
        assert!(!shutting_down.is_finished());
        let _ = finish.send(());
        assert_eq!(
            waiting.await.unwrap().unwrap_err(),
            Error::request_cancelled()
        );
        shutting_down.await.unwrap();
    }
}
