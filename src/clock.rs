use std::cell::Cell;
use std::time::{SystemTime, UNIX_EPOCH};

/// Anything that can report "now" as milliseconds since an arbitrary epoch.
/// Injectable so TTL logic can be tested without sleeping.
pub trait Clock {
    fn now_millis(&self) -> u64;
}

/// Real wall clock, backed by `SystemTime`.
#[derive(Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_millis(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

/// A clock whose value is set by hand. Used by tests and by anything that
/// wants deterministic time control.
pub struct TestClock {
    now: Cell<u64>,
}

impl TestClock {
    pub fn new(start_millis: u64) -> Self {
        TestClock {
            now: Cell::new(start_millis),
        }
    }

    pub fn set(&self, millis: u64) {
        self.now.set(millis);
    }

    pub fn advance(&self, millis: u64) {
        self.now.set(self.now.get().saturating_add(millis));
    }
}

impl Clock for TestClock {
    fn now_millis(&self) -> u64 {
        self.now.get()
    }
}
