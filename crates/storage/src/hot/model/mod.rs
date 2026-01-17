mod db_traits;
pub use db_traits::{HotDbRead, HotDbWrite, HotHistoryRead, HotHistoryWrite};

mod error;
pub use error::{HotKvError, HotKvReadError, HotKvResult};

mod revm;
pub use revm::{RevmRead, RevmWrite};

mod traits;
pub use traits::{HotKv, HotKvRead, HotKvWrite};

mod traverse;
pub use traverse::{
    DualKeyedTraverse, DualTableCursor, DualTableTraverse, KvTraverse, KvTraverseMut, TableCursor,
    TableTraverse, TableTraverseMut,
};

use crate::tables::{DualKeyed, Table};
use std::borrow::Cow;

/// A key-value pair from a table.
pub type GetManyItem<'a, T> = (&'a <T as Table>::Key, Option<<T as Table>::Value>);

/// A key-value tuple from a table.
pub type KeyValue<T> = (<T as Table>::Key, <T as Table>::Value);

/// A raw key-value pair.
pub type RawKeyValue<'a> = (Cow<'a, [u8]>, RawValue<'a>);

/// A raw value.
pub type RawValue<'a> = Cow<'a, [u8]>;

/// A raw dual key-value tuple.
pub type RawDualKeyValue<'a> = (Cow<'a, [u8]>, RawValue<'a>, RawValue<'a>);

/// A dual key-value tuple from a table.
pub type DualKeyValue<T> = (<T as Table>::Key, <T as DualKeyed>::Key2, <T as Table>::Value);
