#[macro_use]
mod macros;

/// Tables that are not hot.
pub mod cold;

/// Tables that are hot, or conditionally hot.
pub mod hot;

use crate::{
    hot::model::{DualKeyValue, KeyValue},
    ser::{DeserError, KeySer, ValSer},
};

/// The maximum size of a dual key (in bytes).
pub const MAX_FIXED_VAL_SIZE: usize = 64;

/// Trait for table definitions.
pub trait Table {
    /// A short, human-readable name for the table.
    const NAME: &'static str;

    /// Indicates that this table uses dual keys.
    const DUAL_KEY: bool = false;

    /// True if the table is guaranteed to have fixed-size values, false
    /// otherwise.
    const FIXED_VAL_SIZE: Option<usize> = None;

    /// Indicates that this table has fixed-size values.
    const IS_FIXED_VAL: bool = Self::FIXED_VAL_SIZE.is_some();

    /// Compile-time assertions for the table.
    #[doc(hidden)]
    const ASSERT: () = {
        // Ensure that fixed-size values do not exceed the maximum allowed size.
        if let Some(size) = Self::FIXED_VAL_SIZE {
            assert!(size <= MAX_FIXED_VAL_SIZE, "Fixed value size exceeds maximum allowed size");
        }
    };

    /// The key type.
    type Key: KeySer;
    /// The value type.
    type Value: ValSer;

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
    const ASSERT: () = {
        assert!(!Self::DUAL_KEY, "SingleKey tables must have DUAL_KEY = false");
    };
}

/// Trait for tables with two keys.
///
/// This trait aims to capture tables that use a composite key made up of two
/// distinct parts. This is useful for representing (e.g.) dupsort or other
/// nested map optimizations.
pub trait DualKeyed: Table {
    /// The second key type.
    type Key2: KeySer;

    /// Compile-time assertions for the dual-keyed table.
    #[doc(hidden)]
    const ASSERT: () = {
        assert!(Self::DUAL_KEY, "DualKeyed tables must have DUAL_KEY = true");
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
