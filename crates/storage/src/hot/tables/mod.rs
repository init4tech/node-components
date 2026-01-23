#[macro_use]
mod macros;

/// Tables that are hot, or conditionally hot.
mod definitions;
pub use definitions::*;

use crate::hot::{
    DeserError, KeySer, MAX_FIXED_VAL_SIZE, MAX_KEY_SIZE, ValSer,
    model::{DualKeyValue, KeyValue},
};

/// Trait for table definitions.
///
/// Tables are compile-time definitions of key-value pairs stored in hot
/// storage. Each table defines the key and value types it uses, along with
/// a name, and information that backends can use for optimizations (e.g.,
/// whether the key or value is fixed-size).
///
/// Tables can be extended to support dual keys by implementing the [`DualKey`]
/// trait. This indicates that the table uses a composite key made up of two
/// distinct parts. Backends can then optimize storage and retrieval of values
/// based on the dual keys.
///
/// Tables that do not implement [`DualKey`] are considered single-keyed tables.
/// Such tables MUST implement the [`SingleKey`] marker trait to indicate that
/// they use a single key. The [`SingleKey`] and [`DualKey`] traits are
/// incompatible, and a table MUST implement exactly one of them.
pub trait Table: Sized + Send + Sync + 'static {
    /// A short, human-readable name for the table.
    const NAME: &'static str;

    /// The key type.
    type Key: KeySer;

    /// The value type.
    type Value: ValSer;

    /// Indicates that this table uses dual keys.
    const DUAL_KEY: bool = Self::DUAL_KEY_SIZE.is_some();

    /// True if the table is guaranteed to have fixed-size values of size
    /// [`MAX_FIXED_VAL_SIZE`] or less, false otherwise.
    const FIXED_VAL_SIZE: Option<usize> = {
        match <Self::Value as ValSer>::FIXED_SIZE {
            Some(size) if size <= MAX_FIXED_VAL_SIZE => Some(size),
            _ => None,
        }
    };

    /// If the table uses dual keys, this is the size of the second key.
    /// Otherwise, it is `None`.
    const DUAL_KEY_SIZE: Option<usize> = None;

    /// Indicates that this table uses an integer key (u32 or u64).
    const INT_KEY: bool = false;

    /// Indicates that this table has fixed-size values.
    const IS_FIXED_VAL: bool = Self::FIXED_VAL_SIZE.is_some();

    /// Compile-time assertions for the table.
    #[doc(hidden)]
    const ASSERT: sealed::Seal = {
        // Ensure that fixed-size values do not exceed the maximum allowed size.
        if let Some(size) = Self::FIXED_VAL_SIZE {
            assert!(size <= MAX_FIXED_VAL_SIZE, "Fixed value size exceeds maximum allowed size");
        }

        if let Some(dual_key_size) = Self::DUAL_KEY_SIZE {
            assert!(Self::DUAL_KEY, "DUAL_KEY_SIZE is set but DUAL_KEY is false");
            assert!(dual_key_size > 0, "DUAL_KEY_SIZE must be greater than zero");
            assert!(dual_key_size <= MAX_KEY_SIZE, "DUAL_KEY_SIZE exceeds maximum key size");
        } else {
            assert!(!Self::DUAL_KEY, "DUAL_KEY is true but DUAL_KEY_SIZE is None");
        }

        assert!(std::mem::size_of::<Self>() == 0, "Table types must be zero-sized types (ZSTs).");

        sealed::Seal
    };

    /// Shortcut to decode a key.
    fn decode_key(data: impl AsRef<[u8]>) -> Result<Self::Key, DeserError> {
        <Self::Key as KeySer>::decode_key(data.as_ref())
    }

    /// Shortcut to decode a value.
    fn decode_value(data: impl AsRef<[u8]>) -> Result<Self::Value, DeserError> {
        <Self::Value as ValSer>::decode_value(data.as_ref())
    }

    /// Shortcut to decode a key-value pair.
    fn decode_kv(
        key_data: impl AsRef<[u8]>,
        value_data: impl AsRef<[u8]>,
    ) -> Result<KeyValue<Self>, DeserError> {
        let key = Self::decode_key(key_data)?;
        let value = Self::decode_value(value_data)?;
        Ok((key, value))
    }

    /// Shortcut to decode a key-value tuple.
    fn decode_kv_tuple(
        data: (impl AsRef<[u8]>, impl AsRef<[u8]>),
    ) -> Result<KeyValue<Self>, DeserError> {
        Self::decode_kv(data.0, data.1)
    }
}

/// Trait for tables with a single key.
pub trait SingleKey: Table {
    /// Compile-time assertions for the single-keyed table.
    #[doc(hidden)]
    const ASSERT: sealed::Seal = {
        assert!(!Self::DUAL_KEY, "SingleKey tables must have DUAL_KEY = false");
        sealed::Seal
    };
}

/// Describes the semantics of a dual-keyed table.
///
///
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DualTableMode {
    /// The table is updated via random access to subkeys.
    ///
    /// This means that there will be random reads/writes to individual
    /// subkeys within a parent key.
    SubkeyAccess,
    /// The table is updated via full key-value replacements. This means that
    /// subkeys within a parent key are not accessed individually, but rather
    /// the entire value is read or replaced at once.
    FullReplacements,
}

/// Trait for tables with two keys.
///
/// This trait aims to capture tables that use a composite key made up of two
/// distinct parts. This is useful for representing (e.g.) dupsort or other
/// nested map optimizations.
pub trait DualKey: Table {
    /// The second key type.
    type Key2: KeySer;

    /// The mode of the dual-keyed table.
    const TABLE_MODE: DualTableMode = DualTableMode::SubkeyAccess;

    /// Compile-time assertions for the dual-keyed table.
    #[doc(hidden)]
    const ASSERT: sealed::Seal = {
        assert!(Self::DUAL_KEY, "DualKeyed tables must have DUAL_KEY = true");
        sealed::Seal
    };

    /// Shortcut to decode the second key.
    fn decode_key2(data: impl AsRef<[u8]>) -> Result<Self::Key2, DeserError> {
        <Self::Key2 as KeySer>::decode_key(data.as_ref())
    }

    /// Shortcut to decode a prepended value. This is useful for some table
    /// implementations.
    fn decode_prepended_value(
        data: impl AsRef<[u8]>,
    ) -> Result<(Self::Key2, Self::Value), DeserError> {
        let data = data.as_ref();
        let key = Self::decode_key2(&data[..Self::Key2::SIZE])?;
        let value = Self::decode_value(&data[Self::Key2::SIZE..])?;
        Ok((key, value))
    }

    /// Shortcut to decode a dual key-value triplet.
    fn decode_kkv(
        key1_data: impl AsRef<[u8]>,
        key2_data: impl AsRef<[u8]>,
        value_data: impl AsRef<[u8]>,
    ) -> Result<DualKeyValue<Self>, DeserError> {
        let key1 = Self::decode_key(key1_data)?;
        let key2 = Self::decode_key2(key2_data)?;
        let value = Self::decode_value(value_data)?;
        Ok((key1, key2, value))
    }

    /// Shortcut to decode a dual key-value tuple.
    fn decode_kkv_tuple(
        data: (impl AsRef<[u8]>, impl AsRef<[u8]>, impl AsRef<[u8]>),
    ) -> Result<DualKeyValue<Self>, DeserError> {
        Self::decode_kkv(data.0, data.1, data.2)
    }
}

mod sealed {
    /// Sealed struct to prevent overriding the `Table::ASSERT` constants.
    #[allow(
        dead_code,
        unreachable_pub,
        missing_copy_implementations,
        missing_debug_implementations
    )]
    pub struct Seal;
}
