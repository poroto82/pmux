//! Client library: IPC, attach, terminal paint.
//!
//! Window: crate `pmux-ui` (`ui/egui`, bin `pmux`). Runtime: `pmux-runtime` (`pwctl`).

pub mod action;
pub mod attach;
pub mod component;
pub mod files;
pub mod ids;
pub mod ipc;
pub mod ipc_client;
pub mod layout;
pub mod markdown;
pub mod monitor;
pub mod palette;
pub mod paths;
pub mod terminal;
pub mod token;
pub mod view;
pub mod widgets;
