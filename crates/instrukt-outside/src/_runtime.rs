//! Shared Tokio runtime bridge for synchronous callers invoking async SDK
//! clients (`rmcp`, `octocrab`, `slack-morphism`, `reqwest`).
use std::future::Future;
use std::io;
use std::sync::OnceLock;

use tokio::runtime::{Builder, Handle, Runtime, RuntimeFlavor};

static SHARED_RUNTIME: OnceLock<Result<Runtime, String>> = OnceLock::new();

/// Returns the process-wide multi-thread Tokio runtime, building it lazily on
/// first use.
pub fn runtime() -> io::Result<&'static Runtime> {
    match SHARED_RUNTIME.get_or_init(|| {
        Builder::new_multi_thread()
            .enable_all()
            .thread_name("instrukt-rt")
            .build()
            .map_err(|error| error.to_string())
    }) {
        Ok(rt) => Ok(rt),
        Err(message) => Err(io::Error::other(format!(
            "cannot start Tokio runtime: {message}"
        ))),
    }
}

/// Runs `future` to completion from synchronous code.
///
/// Inside a multi-thread Tokio runtime this uses [`tokio::task::block_in_place`].
/// Inside a current-thread Tokio runtime blocking would pause the sole executor
/// thread and risk deadlock, so it returns an error instead — async callers
/// should await directly.
pub fn block_on<F>(future: F) -> io::Result<F::Output>
where
    F: Future + Send,
    F::Output: Send,
{
    if let Ok(handle) = Handle::try_current() {
        return match handle.runtime_flavor() {
            RuntimeFlavor::MultiThread => {
                Ok(tokio::task::block_in_place(|| handle.block_on(future)))
            }
            _ => Err(io::Error::other(
                "cannot block_on from a current-thread Tokio runtime; await directly",
            )),
        };
    }
    Ok(runtime()?.block_on(future))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_on_rejects_current_thread_runtime_instead_of_deadlocking() {
        let rt = Builder::new_current_thread().enable_all().build().unwrap();
        let err = rt
            .block_on(async { block_on(async { 42u8 }) })
            .expect_err("current-thread runtime must refuse block_on");
        assert!(
            err.to_string().contains("current-thread"),
            "unexpected error: {err}"
        );
    }
}
