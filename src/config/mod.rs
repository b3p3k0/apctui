// SPDX-License-Identifier: GPL-3.0-or-later
//! Config subsystem: round-trip parsing, typed schema, validation, diffing.

pub mod diff;
pub mod parser;
pub mod schema;
pub mod validate;

pub use parser::ConfigFile;
pub use schema::{Group, Kind};
pub use validate::{validate, Finding, Severity};
