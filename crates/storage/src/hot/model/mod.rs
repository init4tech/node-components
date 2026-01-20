//! Hot storage models and traits.
//!
//! The core trait for the hot storage module is [`HotKv`], which provides
//! a transaction factory for creating read and write transactions. The
//! [`HotKvRead`] and [`HotKvWrite`] traits provide transactional read-only and
//! read-write access to hot storage, respectively. The [`HotDbRead`] and
//! [`HotDbWrite`] traits provide higher-level abstractions for working with
//! predefined tables and their associated key and value types.
//!
//! ## Dual-Keyed Tables
//!
//! The hot storage module supports dual-keyed tables, which allow for
//! storing values associated with a combination of two keys. The [`DualKey`]
//! trait defines the interface for dual keys. Dual-keying is a common
//! optimization in KV stores like MDBX and RocksDB, allowing for efficient
//! storage and retrieval of values based on composite keys.
//!
//! [`HotKvRead`] and [`HotKvWrite`] provide methods for working with dual-keyed
//! tables, including getting, setting, and deleting values based on dual keys.
//!
//! ## Traversal
//!
//! The hot storage module provides traversal abstractions for iterating
//! over key-value pairs in tables. The [`KvTraverse`] and [`KvTraverseMut`]
//! traits provide methods for traversing single-keyed tables, while the
//! [`DualTableTraverse`] and [`DualKeyTraverse`] traits provide methods for
//! traversing dual-keyed tables.
//!
//! These traversal traits allow for efficient iteration over key-value pairs,
//! supporting operations like seeking to specific keys, moving to the next or
//! previous entries, and retrieving the current key-value pair. These are then
//! extended with the [`TableTraverse`], [`TableTraverseMut`],
//! and [`DualTableTraverse`] to provide automatic (de)serialization of keys
//! and values.
//!
//! The library wraps these into the [`TableCursor`] and [`DualTableCursor`]
//! structs for ease of use and consistency across different backends.

mod db_traits;
pub use db_traits::{HotDbRead, HotDbWrite, HotHistoryRead, HotHistoryWrite};

mod error;
pub use error::{HotKvError, HotKvReadError, HotKvResult};

mod revm;
pub use revm::{RevmRead, RevmWrite};

mod traits;
pub use traits::{HotKv, HotKvRead, HotKvWrite};

mod traverse;
pub use traverse::{
    DualKeyTraverse, DualTableCursor, DualTableTraverse, KvTraverse, KvTraverseMut, TableCursor,
    TableTraverse, TableTraverseMut,
};

use crate::hot::tables::{DualKey, Table};
use std::borrow::Cow;

/// A key-value pair from a table.
pub type GetManyItem<'a, T> = (&'a <T as Table>::Key, Option<<T as Table>::Value>);

/// A key-value tuple from a table.
pub type KeyValue<T> = (<T as Table>::Key, <T as Table>::Value);

/// A raw key-value pair.
pub type RawKeyValue<'a> = (Cow<'a, [u8]>, RawValue<'a>);

/// A raw value.
pub type RawValue<'a> = Cow<'a, [u8]>;

/// A raw dual key-value tuple.
pub type RawDualKeyValue<'a> = (Cow<'a, [u8]>, RawValue<'a>, RawValue<'a>);

/// A dual key-value tuple from a table.
pub type DualKeyValue<T> = (<T as Table>::Key, <T as DualKey>::Key2, <T as Table>::Value);
