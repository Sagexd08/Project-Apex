use atlas_engine::Engine;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub const TICK: Duration = Duration::from_millis(250);
pub const PENDING_LIMIT: u64 = 20_000;
const IDLE_TICKS: u32 = 2;

pub fn due(pending: u64, idle_ticks: u32) -> bool {
    pending >= PENDING_LIMIT || (pending > 0 && idle_ticks >= IDLE_TICKS)
}

pub fn spawn(engine: Arc<Engine>, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut last_seen = engine.applied_seq();
        let mut idle = 0u32;
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(TICK);
            let now = engine.applied_seq();
            idle = if now == last_seen { idle + 1 } else { 0 };
            last_seen = now;
            if !due(now.saturating_sub(engine.checkpointed_seq()), idle) {
                continue;
            }
            match engine.checkpoint() {
                Ok(_) => {}
                Err(e) => eprintln!("checkpoint failed: {e}"),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_pending_is_never_due() {
        assert!(!due(0, 100));
    }

    #[test]
    fn busy_small_backlog_waits() {
        assert!(!due(PENDING_LIMIT - 1, 0));
        assert!(!due(5, IDLE_TICKS - 1));
    }

    #[test]
    fn idle_backlog_or_big_backlog_is_due() {
        assert!(due(5, IDLE_TICKS));
        assert!(due(PENDING_LIMIT, 0));
    }
}
