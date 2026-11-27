use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// One-shot value slot. Readers can poll or wait for a bounded interval.
pub struct Latch<T: Clone> {
    inner: Mutex<Option<T>>,
    cv: Condvar,
}

impl<T: Clone> Latch<T> {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
            cv: Condvar::new(),
        }
    }

    pub fn set(&self, val: T) {
        let mut guard = self.inner.lock().unwrap();
        *guard = Some(val);
        self.cv.notify_all();
    }

    /// Return the value if it arrives within `timeout`.
    pub fn wait_timeout(&self, timeout: std::time::Duration) -> Option<T> {
        let guard = self.inner.lock().unwrap();
        let (guard, _) = self
            .cv
            .wait_timeout_while(guard, timeout, |v| v.is_none())
            .unwrap();
        guard.as_ref().cloned()
    }

    pub fn try_get(&self) -> Option<T> {
        let guard = self.inner.lock().unwrap();
        guard.as_ref().cloned()
    }
}

/// Panic-safe replacement for `std::sync::Barrier`. `signal_all` wakes
/// every thread blocked in `wait`; if a worker panics before reaching
/// `wait`, the remaining workers and the signaler aren't deadlocked.
pub struct VisibleLatch {
    inner: Mutex<bool>,
    cv: Condvar,
}

impl VisibleLatch {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(false),
            cv: Condvar::new(),
        }
    }

    pub fn signal_all(&self) {
        let mut guard = self.inner.lock().unwrap();
        *guard = true;
        self.cv.notify_all();
    }

    pub fn wait(&self) {
        let guard = self.inner.lock().unwrap();
        let _guard = self
            .cv
            .wait_while(guard, |signaled| !*signaled)
            .unwrap();
    }
}

/// Report an unexpected render-worker exit, including an unwinding panic.
pub struct RenderFailureGuard {
    counter: Arc<AtomicUsize>,
    window: Option<Arc<winit::window::Window>>,
    armed: bool,
}

impl RenderFailureGuard {
    pub fn new(counter: Arc<AtomicUsize>) -> Self {
        Self {
            counter,
            window: None,
            armed: true,
        }
    }

    pub fn attach_window(&mut self, window: Arc<winit::window::Window>) {
        self.window = Some(window);
    }

    pub fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for RenderFailureGuard {
    fn drop(&mut self) {
        if self.armed {
            self.counter.fetch_add(1, Ordering::Release);
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_unexpected_worker_exits_are_reported() {
        let failures = Arc::new(AtomicUsize::new(0));
        let mut stopped = RenderFailureGuard::new(failures.clone());
        stopped.disarm();
        drop(stopped);
        assert_eq!(failures.load(Ordering::Acquire), 0);

        drop(RenderFailureGuard::new(failures.clone()));
        assert_eq!(failures.load(Ordering::Acquire), 1);
    }
}
