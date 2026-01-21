mod error;
pub use error::DeserError;

mod impls;

mod reth_impls;

mod traits;
pub use traits::{KeySer, MAX_FIXED_VAL_SIZE, MAX_KEY_SIZE, ValSer};
