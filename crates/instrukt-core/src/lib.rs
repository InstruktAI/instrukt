//! instrukt core: the always-on module — docs, configuration, publication,
//! repository and specs — plus the shared identifiers, home layout, effects
//! and the interfaces core declares for optional modules.
//!
//! Dependencies point one way: core depends on nothing of instrukt.

pub mod config;
pub mod docs;
pub mod fsx;
pub mod git;
pub mod home;
pub mod ids;
pub mod index;
pub mod interfaces;
pub mod publication;
pub mod repository;
pub mod specs;
