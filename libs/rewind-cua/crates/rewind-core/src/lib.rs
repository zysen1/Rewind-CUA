//! Rewind-CUA core: journal, policy, uncertainty escalation, checkpoints.
//!
//! Built on trycua/cua. This crate does not modify the upstream sandboxes.
//! It snapshots a workspace directory by default and can shell out to a
//! caller-supplied command for a real VM checkpoint. See `docs/SNAPSHOTS.md`.

#![forbid(unsafe_code)]

pub mod action;
pub mod demo;
pub mod error;
pub mod escalation;
pub mod eval;
pub mod harness;
pub mod journal;
pub mod policy;
pub mod snapshot;
pub mod world;

pub use error::Error;
