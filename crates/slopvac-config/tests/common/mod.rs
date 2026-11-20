//! Shared helpers for the integration tests.

#![allow(
    dead_code,
    unreachable_pub,
    missing_docs,
    reason = "shared test support; each test binary uses some of it"
)]

/// A [`Presets`](slopvac_config::Presets) with no packs behind it, for tests that cover the
/// loader alone. Real packs live in `slopvac-packs`, which depends on this crate.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoPresets;

impl slopvac_config::Presets for NoPresets {
    fn style(&self, _name: &str) -> Option<Vec<slopvac_config::Setting>> {
        None
    }

    fn genre_chain(&self, _name: &str) -> Vec<Vec<slopvac_config::Setting>> {
        Vec::new()
    }

    fn pack_config(&self, _name: &str) -> Option<slopvac_config::PackFile> {
        None
    }
}

/// Run `f` on a worker thread and return its result, failing the test when it blocks.
///
/// - The worker ends early without a result (a panic): join at once and re-raise its panic.
/// - Five seconds pass with no result: open the FIFO's write end without blocking, which wakes a
///   reader stuck in `open`. `ENXIO` means no reader is waiting, so there is nothing to wake.
///   Then wait once more, join the worker and fail; a worker that is still stuck is left behind,
///   so recovery never hangs.
pub fn within_timeout<T: Send + 'static>(
    fifo: &std::path::Path,
    f: impl FnOnce() -> T + Send + 'static,
) -> T {
    use std::os::unix::fs::OpenOptionsExt;
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::time::Duration;

    const O_NONBLOCK: i32 = if cfg!(target_os = "linux") {
        0o4000
    } else {
        0x4
    };
    const LIMIT: Duration = Duration::from_secs(5);

    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(LIMIT) {
        Ok(value) => {
            worker.join().expect("the worker finished");
            value
        }
        Err(RecvTimeoutError::Disconnected) => match worker.join() {
            Err(panic) => std::panic::resume_unwind(panic),
            Ok(()) => panic!("the worker ended without a result"),
        },
        Err(RecvTimeoutError::Timeout) => {
            let _writer = std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(O_NONBLOCK)
                .open(fifo);
            match rx.recv_timeout(LIMIT) {
                Ok(_) | Err(RecvTimeoutError::Disconnected) => {
                    let _ = worker.join();
                    panic!("the read blocked on a FIFO until it was woken");
                }
                Err(RecvTimeoutError::Timeout) => panic!("the read blocked on a FIFO"),
            }
        }
    }
}
