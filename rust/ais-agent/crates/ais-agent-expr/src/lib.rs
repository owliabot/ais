//! Local expression layer for the greenfield `ais-agent`.
//!
//! This crate is intentionally constrained:
//! - derivations
//! - local policy predicates
//! - effect verification predicates
//! - readiness or boundary predicates
//!
//! It is explicitly not the planning language or orchestration language.

pub mod cel;
