#[macro_use]
mod macros;

/// Tables that are not hot.
pub mod cold;

/// Tables that are hot, or conditionally hot.
pub mod hot;

use crate::ser::{KeySer, ValSer};

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
}
