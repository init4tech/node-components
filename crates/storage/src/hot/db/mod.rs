//! Primary access for hot storage backends.
//!
//!
//!

mod consistent;
pub use consistent::HistoryWrite;

mod errors;
pub use errors::{HistoryError, HistoryResult};

mod inconsistent;
pub use inconsistent::{UnsafeDbWrite, UnsafeHistoryWrite};

mod read;
pub use read::{HotDbRead, HotHistoryRead};

pub(crate) mod sealed {
    use crate::hot::model::HotKvRead;

    /// Sealed trait to prevent external implementations of HotDbReader and HotDbWriter.
    #[allow(dead_code, unreachable_pub)]
    pub trait Sealed {}
    impl<T> Sealed for T where T: HotKvRead {}
}
