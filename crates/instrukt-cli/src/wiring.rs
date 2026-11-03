//! Wiring of optional modules into the interfaces core and the runner
//! declare. Only this file decides which module implementation answers;
//! it wires a module only when its switch is on.

use instrukt_core::interfaces::CoreWiring;

use crate::context::Ctx;

/// The optional-module implementations for core contexts in this
/// invocation. A module that is off leaves its slot empty.
#[allow(
    dead_code,
    reason = "wired as core commands start consuming optional modules"
)]
pub fn core_wiring(_ctx: &Ctx) -> CoreWiring {
    CoreWiring::default()
}
