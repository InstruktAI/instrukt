//! instrukt core: the always-on module — docs, configuration, publication,
//! repository and specs — plus the shared identifiers, home layout, effects
//! and the interfaces core declares for optional modules.
//!
//! Dependencies point one way: core depends on nothing of instrukt.
#![cfg_attr(
    test,
    allow(
        clippy::disallowed_methods,
        reason = "unit tests build their scratch trees directly"
    )
)]

pub mod config;
pub mod docs;
pub mod event_type;
pub mod fsx;
pub mod git;
pub mod github;
pub mod home;
pub mod ids;
pub mod index;
pub mod interfaces;
pub mod machine;
pub mod modeline;
pub mod publication;
pub mod repository;
pub mod specs;
pub mod state_db;

pub mod subscription;

pub mod process;

pub mod capabilities;

#[cfg(all(instrukt_release_build, debug_assertions))]
compile_error!("INSTRUKT_RELEASE_BUILD=1 cannot be enabled when debug_assertions are active");

/// Whether this binary was compiled as a release build (`INSTRUKT_RELEASE_BUILD=1`).
///
/// Only the release workflow sets `INSTRUKT_RELEASE_BUILD=1`; every local
/// `cargo build`, `cargo test`, `cargo run`, and local `cargo build --release`
/// compiles as a development build.
pub const fn is_release_build() -> bool {
    cfg!(instrukt_release_build)
}

/// The installer command printed when stored state requires a newer release.
pub const INSTALLER_COMMAND: &str =
    "curl -fsSL https://github.com/InstruktAI/instrukt/releases/latest/download/install.sh | sh";
