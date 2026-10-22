//! A non-blocking, dual duration/bytes reservation. Audio overload never blocks control.
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub struct AudioBudget {
    max_us: u64,
    max_bytes: usize,
    used: Mutex<(u64, usize)>,
}

impl AudioBudget {
    pub fn new(max_ms: u64, max_bytes: usize) -> Arc<Self> {
        Arc::new(Self {
            max_us: max_ms.saturating_mul(1000),
            max_bytes,
            used: Mutex::new((0, 0)),
        })
    }
    pub fn reserve(self: &Arc<Self>, us: u64, bytes: usize) -> Option<AudioPermit> {
        let mut used = self.used.lock().expect("audio budget lock");
        let next = (used.0.checked_add(us)?, used.1.checked_add(bytes)?);
        if next.0 > self.max_us || next.1 > self.max_bytes {
            return None;
        }
        *used = next;
        Some(AudioPermit {
            budget: self.clone(),
            us,
            bytes,
        })
    }
    #[cfg(test)]
    pub fn usage(&self) -> (u64, usize) {
        *self.used.lock().unwrap()
    }
}

#[derive(Debug)]
pub struct AudioPermit {
    budget: Arc<AudioBudget>,
    us: u64,
    bytes: usize,
}
impl Drop for AudioPermit {
    fn drop(&mut self) {
        let mut used = self.budget.used.lock().expect("audio budget lock");
        used.0 -= self.us;
        used.1 -= self.bytes;
    }
}

#[cfg(test)]
mod tests;
