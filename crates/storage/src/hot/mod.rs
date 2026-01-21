//! Hot storage module.
//!
//! Hot storage is designed for fast read and write access to frequently used
//! data. It provides abstractions and implementations for key-value storage
//! backends.
//!
//! ## Serialization
//!
//! Hot storage is opinionated with respect to serialization. Each table defines
//! the key and value types it uses, and these types must implement the
//! appropriate serialization traits. See the [`KeySer`] and [`ValSer`] traits
//! for more information.
//!
//! # Trait Model
//!
//! The hot storage module defines a set of traits to abstract over different
//! hot storage backends. The primary traits are:
//!
//! - [`HotKvRead`]: for transactional read-only access to hot storage.
//! - [`HotKvWrite`]: for transactional read-write access to hot storage.
//! - [`HotKv`]: for creating read and write transactions.
//!
//! These traits provide methods for common operations such as getting,
//! setting, and deleting key-value pairs in hot storage tables. The raw
//! key-value operations use byte slices for maximum flexibility. The
//! [`HotDbRead`] and [`HotDbWrite`] traits provide higher-level abstractions
//! that work with the predefined tables and their associated key and value
//! types.
//!
//! See the [`model`] module documentation for more details on the traits and
//! their usage.
//!
//! ## Tables
//!
//! Hot storage tables are predefined in the [`tables`] module. Each table
//! defines the key and value types it uses, along with serialization logic.
//! The [`Table`] and [`DualKey`] traits define the interface for tables.
//! The [`SingleKey`] trait is a marker for tables with single keys.
//!
//! See the [`Table`] trait documentation for more information on defining and
//! using tables.
//!
//! [`HotDbRead`]: crate::hot::model::HotDbRead
//! [`HotDbWrite`]: crate::hot::model::HotDbWrite
//! [`HotKvRead`]: crate::hot::model::HotKvRead
//! [`HotKvWrite`]: crate::hot::model::HotKvWrite
//! [`HotKv`]: crate::hot::model::HotKv
//! [`DualKey`]: crate::hot::tables::DualKey
//! [`SingleKey`]: crate::hot::tables::SingleKey
//! [`Table`]: crate::hot::tables::Table

/// Conformance tests for hot storage backends.
#[cfg(any(test, feature = "test-utils"))]
pub mod conformance;

pub mod db;

pub mod model;

/// Implementations of hot storage backends.
#[cfg(feature = "impls")]
pub mod impls;

/// Serialization module.
pub mod ser;
pub use ser::{DeserError, KeySer, MAX_FIXED_VAL_SIZE, MAX_KEY_SIZE, ValSer};

/// Predefined tables module.
pub mod tables;
