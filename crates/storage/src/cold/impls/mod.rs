//! Cold storage backend implementations.
//!
//! This module contains implementations of the [`ColdStorage`] trait
//! for various backends.

#[cfg(any(test, feature = "in-mem"))]
pub mod mem;
