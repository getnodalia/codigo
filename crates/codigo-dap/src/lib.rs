// SPDX-License-Identifier: MIT OR Apache-2.0
//! Código's debugger, as a library with no interface.
//!
//! [`client`] speaks the Debug Adapter Protocol over any pair of byte streams. [`adapters`] knows
//! which adapters exist for which languages (lldb-dap and gdb, debugpy, Delve) and never installs
//! one. [`target`] works out what to debug in a folder and builds it. [`session`] runs a whole
//! debugging session on its own thread: the window sends it [`Command`]s and reads [`Update`]s.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod adapters;
pub mod client;
pub mod session;
pub mod target;

pub use adapters::{adapter_for, adapters_for, Adapter, AdapterKind, Missing};
pub use client::{Client, ClientError, Incoming, Pending};
pub use session::{Command, Debugger, Frame, LaunchSpec, ScopeVars, StopInfo, Update, Variable};
pub use target::{language_of, prepare, save, saved, split_args, Prepare, Saved, Target};
