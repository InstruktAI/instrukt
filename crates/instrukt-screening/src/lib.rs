//! The screening bounded context (`docs/project/spec/domain/screening.md`):
//!
//! - [`call`] / [`call_async`] — internal reusable capability-free model client
//!   making one OpenAI-compatible `POST {url}/chat/completions` request with
//!   `tools` omitted, no earlier conversation, and a bounded completion size.
//! - [`is_safe`] / [`is_safe_async`] — stateless safety screening of untrusted
//!   text over CSPRNG-shuffled 3-to-8-word fragments through two independent
//!   capability-free checks.
//!
//! Screening creates no local database, lock file, or duplicate copy of
//! screened text. Every HTTP try routes through the host-supplied
//! [`instrukt_outside::Gate`].

mod _client;
mod _fragments;
mod _screen;

pub use _client::{
    CONNECT_TIMEOUT, CallDefect, CallOutcome, CallRefusal, DEFAULT_MAX_TOKENS, EntryRefusal,
    MAX_MAX_TOKENS, MAX_RESPONSE_BODY_BYTES, MIN_MAX_TOKENS, call, call_async,
};
pub use _screen::{MAX_REASON_CHARS, SafeOutcome, UnsafeCause, is_safe, is_safe_async};
