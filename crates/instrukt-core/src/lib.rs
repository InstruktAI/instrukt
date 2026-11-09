//! instrukt core: the always-on module — docs, configuration, publication,
//! repository and specs — plus the shared identifiers, home layout, effects
//! and the interfaces core declares for optional modules.
//!
//! Dependencies point one way: core depends on nothing of instrukt.

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
