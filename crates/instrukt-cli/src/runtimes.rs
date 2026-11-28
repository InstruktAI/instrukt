//! The runtime adapters this binary is built with: each runtime's declaration,
//! installed into configuration before anything reads it, and the shared
//! `RuntimeRegistry` factory that resolves configured runtimes to concrete
//! adapters. Only `instrukt-cli` links concrete runtime adapter crates.
use std::sync::Arc;

use instrukt_core::config::{Env, RuntimeDeclaration};
use instrukt_runtime::{RuntimeAdapter, RuntimeError, RuntimeFactory, RuntimeRegistry};
use instrukt_runtime_t3::{T3_DECLARATION, T3Factory};

/// Every compiled runtime declaration of this build.
pub static DECLARATIONS: [&RuntimeDeclaration; 1] = [&T3_DECLARATION];

/// Build the `RuntimeRegistry` holding every compiled runtime factory.
pub fn registry() -> RuntimeRegistry {
    RuntimeRegistry::new().with(Arc::new(T3Factory) as Arc<dyn RuntimeFactory>)
}

/// Construct a one-shot adapter for `runtime_name` under `env`.
pub fn adapter(runtime_name: &str, env: &Env) -> Result<Arc<dyn RuntimeAdapter>, RuntimeError> {
    registry().adapter(runtime_name, env)
}
