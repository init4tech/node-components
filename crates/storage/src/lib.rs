//! Signet Storage Components
//!
//! High-level abstractions and implementations for storage backends used in
//! Signet.
//!
//! ## Design Overview
//!
//! We divide storage into two main categories: cold storage and hot storage.
//! The distinction is not access patterns, but whether the data is used in the
//! critical consensu s path (hot) or not (cold). Cold storage is used for
//! serving blocks and transactions over RPC, while hot storage is used for
//! fast access to frequently used data during block processing and consensus.
//!
//! The crate has two modules:
//! - [`cold`]: Cold storage abstractions and implementations.
//! - [`hot`]: Hot storage abstractions and implementations.
//!
//! ## Hot Storage
//!
//! Hot storage is modeled as a key-value store with predefined tables. The core
//! trait is [`HotKv`], which provides a factory for creating read and write
//! transactions.
//!
//! The primary traits for accessing hot storage are:
//! - [`HistoryRead`]: for read-only transactions.
//! - [`HistoryWrite`]: for read-write transactions.
//!
//! Other traits should generally only be used when implementing new backends.
//!
//! [`HotKv::revm_reader`] and [`HotKv::revm_writer`] create a transaction
//! wrapper that implements the [`revm`] crate's storage traits, allowing
//! seamless integration with the EVM execution engine.
//!
//! When the "mdbx" flag is enabled, we provide an MDBX-based implementation of
//! the hot storage traits. See the `hot_impls::mdbx` module for more details.
//!
//! ## Cold Storage
//!
//! Cold storage provides abstractions for storing and retrieving blocks,
//! transactions, and related data.
//!
//! Unlike hot storage, cold is intended to be accessed asynchronously. The core
//! trait is [`ColdStorage`], which defines methods for appending and reading
//! from the store.
//!
//! A [`ColdStorage`] implementation is typically run in a separate task using
//! the [`ColdStorageTask`]. The task processes requests sent via a channel,
//! allowing non-blocking access to cold storage operations. The
//! [`ColdStorageHandle`] provides an ergonomic API for sending requests to the
//! task.
//!
//! Like [`hot`], the majority of users will not need to interact with cold
//! storage directly. Instead, they will use the task and handle abstractions.
//!
//! [`revm`]: trevm::revm
#![warn(
    missing_copy_implementations,
    missing_debug_implementations,
    missing_docs,
    unreachable_pub,
    clippy::missing_const_for_fn,
    rustdoc::all
)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![deny(unused_must_use, rust_2018_idioms)]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

/// Cold storage module.
pub mod cold;
pub use cold::{ColdStorage, ColdStorageError, ColdStorageHandle, ColdStorageTask};

#[cfg(feature = "impls")]
pub use cold::impls as cold_impls;

/// Hot storage module.
pub mod hot;
pub use hot::{HistoryError, HistoryRead, HistoryWrite, HotKv};

#[cfg(feature = "impls")]
pub use hot::impls as hot_impls;
