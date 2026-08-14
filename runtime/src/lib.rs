//! Runtime process guts: PTYs, layout source of truth, IPC server.
//!
//! Binary: `pwctl` (`--daemon` / `start`). UI never depends on this crate.

pub mod daemon;
pub mod event;
pub mod ipc_server;
pub mod names;
pub mod pane;
pub mod permission;
pub mod persistence;
pub mod runtime;
pub mod session;
pub mod watch;
pub mod workspace;
