//! Async cold storage engine for historical Ethereum data.
//!
//! This module provides an abstraction over various backend storage systems
//! for historical blockchain data. Unlike hot storage which uses transaction
//! semantics for mutable state, cold storage is optimized for:
//!
//! - **Append-only writes** with block-ordered data
//! - **Efficient bulk reads** by block number or index
//! - **Truncation** (reorg handling) that removes data beyond a certain block
//! - **Index maintenance** for hash-based lookups
//!
//! # Architecture
//!
//! The cold storage engine uses a task-based architecture:
//!
//! - [`ColdStorage`] trait defines the backend interface
//! - [`ColdStorageTask`] processes requests from a channel
//! - [`ColdStorageHandle`] provides an ergonomic API for sending requests
//!
//! # Example
//!
//! ```ignore
//! use tokio_util::sync::CancellationToken;
//! use signet_storage::cold::{ColdStorageTask, impls::MemColdBackend};
//!
//! let cancel = CancellationToken::new();
//! let handle = ColdStorageTask::spawn(MemColdBackend::new(), cancel);
//!
//! // Use the handle to interact with cold storage
//! let header = handle.get_header_by_number(100).await?;
//! ```

mod error;
mod request;
mod specifier;
mod traits;

/// Task module containing the storage task runner and handle.
pub mod task;

/// Backend implementations.
#[cfg(feature = "impls")]
pub mod impls;

/// Conformance tests for cold storage backends.
#[cfg(any(test, feature = "test-utils"))]
pub mod conformance;

pub use error::*;
pub use request::*;
pub use specifier::*;
pub use task::{ColdStorageHandle, ColdStorageTask};
pub use traits::*;
