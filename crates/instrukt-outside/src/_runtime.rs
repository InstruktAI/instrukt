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

/// Runs `future` to completion from synchronous code without panicking when
/// invoked from inside an active Tokio runtime (multi-thread or current-thread).
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
            _ => {
                let rt = runtime()?;
                std::thread::scope(|scope| {
                    scope
                        .spawn(|| rt.block_on(future))
                        .join()
                        .map_err(|_| io::Error::other("async bridge thread panicked"))
                })
            }
        };
    }
    Ok(runtime()?.block_on(future))
}
