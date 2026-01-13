#[macro_use]
mod macros;

/// Tables that are not hot.
pub mod cold;

/// Tables that are hot, or conditionally hot.
pub mod hot;

use crate::ser::{KeySer, ValSer};

/// Trait for table definitions.
pub trait Table {
    /// A Human-readable name for the table.
    const NAME: &'static str;

    /// The key type.
    type Key: KeySer;
    /// The value type.
    type Value: ValSer;
}
