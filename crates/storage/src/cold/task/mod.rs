//! Cold storage task and handle.
//!
//! This module provides the task-based architecture for cold storage:
//!
//! - [`ColdStorageTask`] processes requests from a channel
//! - [`ColdStorageHandle`] provides an ergonomic API for sending requests

mod handle;
pub use handle::ColdStorageHandle;

mod runner;
pub use runner::ColdStorageTask;
