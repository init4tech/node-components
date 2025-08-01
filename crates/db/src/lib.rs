//! Database access for Signet Node.
//!
//! This library contains the following:
//!
//! - Traits for reading and writing Signet events
//! - Table definitions for Signet Events and Headers
//! - Helpers for reading and writing Signet EVM blocks and headers

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

mod chain;
pub use chain::{DbExtractionResults, RuChain};

mod convert;
pub use convert::DataCompat;

mod provider;

mod tables;
pub use tables::{
    DbEnter, DbEnterToken, DbSignetEvent, DbTransact, DbZenithHeader, JournalHashes, SignetEvents,
    ZenithHeaders,
};

mod traits;
pub use traits::{DbProviderExt, RuEnterReader, RuWriter};
