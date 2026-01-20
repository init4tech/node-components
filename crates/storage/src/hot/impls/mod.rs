/// An in-memory key-value store implementation.
#[cfg(any(test, feature = "in-mem"))]
pub mod mem;

/// MDBX-backed key-value store implementation.
#[cfg(feature = "mdbx")]
pub mod mdbx;
