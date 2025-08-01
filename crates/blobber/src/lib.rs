//! Contains logic for extracting data from host chain blocks.

mod block_data;
pub use block_data::{Blobs, BlockExtractor};

mod error;
pub use error::{BlockExtractionError, ExtractionResult};

mod shim;
pub use shim::ExtractableChainShim;
