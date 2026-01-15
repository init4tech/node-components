#[macro_use]
mod macros;

/// Tables that are not hot.
pub mod cold;

/// Tables that are hot, or conditionally hot.
pub mod hot;

use crate::ser::{KeySer, ValSer};

/// Trait for table definitions.
pub trait Table {
    /// A short, human-readable name for the table.
    const NAME: &'static str;

    /// Indicates that this table uses dual keys.
    const DUAL_KEY: bool = false;

    /// Indicates that this table has fixed-size values.
    const DUAL_FIXED_VAL: bool = false;

    /// The key type.
    type Key: KeySer;
    /// The value type.
    type Value: ValSer;
}

/// Trait for tables with two keys.
///
/// This trait aims to capture tables that use a composite key made up of two
/// distinct parts. This is useful for representing (e.g.) dupsort or other
/// nested map optimizations.
pub trait DualKeyed {
    /// A short, human-readable name for the table.
    const NAME: &'static str;

    /// If the value size is fixed, `Some(size)`. Otherwise, `None`.
    const FIXED_VALUE_SIZE: Option<usize> = None;

    /// The first key type.
    type K1: KeySer;

    /// The second key type.
    type K2: KeySer;

    /// The value type.
    type Value: ValSer;
}

impl<T> Table for T
where
    T: DualKeyed,
{
    const NAME: &'static str = T::NAME;

    /// Indicates that this table uses dual keys.
    const DUAL_KEY: bool = true;

    /// Indicates that this table has fixed-size values.
    const DUAL_FIXED_VAL: bool = T::FIXED_VALUE_SIZE.is_some();

    type Key = T::K1;
    type Value = T::Value;
}
