use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

/// 统一时间源，单位 Unix 毫秒（UTC）。
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

#[derive(Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        chrono::Utc::now().timestamp_millis()
    }
}

/// 测试用可控时钟。
#[derive(Clone)]
pub struct FakeClock {
    inner: Arc<AtomicI64>,
}

impl FakeClock {
    pub fn new(ts_ms: i64) -> Self {
        Self {
            inner: Arc::new(AtomicI64::new(ts_ms)),
        }
    }

    pub fn set(&self, ts_ms: i64) {
        self.inner.store(ts_ms, Ordering::SeqCst);
    }

    /// 前进若干毫秒。
    pub fn advance(&self, delta_ms: i64) {
        self.inner.fetch_add(delta_ms, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now_ms(&self) -> i64 {
        self.inner.load(Ordering::SeqCst)
    }
}
