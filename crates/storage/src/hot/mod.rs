/// Hot storage models and traits.
pub mod model;

mod impls;
pub use impls::{mdbx, mem};
