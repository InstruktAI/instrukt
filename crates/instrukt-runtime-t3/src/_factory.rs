//! Construction of one-shot and long-lived concrete adapters.
use std::sync::Arc;

use instrukt_core::config::Env;
use instrukt_runtime::{RuntimeAdapter, RuntimeFactory};

use crate::{T3, T3_DECLARATION};

/// Factory constructing one-shot or long-lived T3 adapters.
#[derive(Debug, Default)]
pub struct T3Factory;

impl RuntimeFactory for T3Factory {
    fn runtime_name(&self) -> &'static str {
        T3_DECLARATION.name
    }

    fn create(&self, env: &Env) -> Arc<dyn RuntimeAdapter> {
        Arc::new(T3::with_env(env.clone()))
    }

    fn create_long_lived(&self, env: &Env) -> Arc<dyn RuntimeAdapter> {
        Arc::new(T3::long_lived(env.clone()))
    }
}
