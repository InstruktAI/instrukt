use std::time::{Duration, Instant};

/// Polls an observable condition without using elapsed time to infer success.
pub fn wait_until(what: &str, bound: Duration, condition: impl FnMut() -> bool) {
    wait_until_with_interval(what, bound, Duration::from_millis(10), condition);
}

pub fn wait_until_with_interval(
    what: &str,
    bound: Duration,
    interval: Duration,
    mut condition: impl FnMut() -> bool,
) {
    let deadline = Instant::now() + bound;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(interval);
    }
}
