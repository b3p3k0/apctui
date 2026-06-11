// SPDX-License-Identifier: GPL-3.0-or-later
//! apctui library surface — exposes modules for integration testing and
//! potential reuse. The binary (main.rs) is a thin CLI/event-loop shell.

pub mod app;
pub mod clientgen;
pub mod config;
pub mod events;
pub mod netutil;
pub mod nis;
pub mod notify;
pub mod options;
pub mod poller;
pub mod privileged;
pub mod registry;
pub mod service;
pub mod theme;
pub mod ui;
