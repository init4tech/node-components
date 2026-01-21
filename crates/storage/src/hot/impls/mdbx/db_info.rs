use crate::hot::ValSer;
use reth_libmdbx::DatabaseFlags;

/// Type alias for the database info cache.
pub type DbCache = std::sync::Arc<dashmap::DashMap<&'static str, DbInfo>>;

/// Information about fixed size values in a database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixedSizeInfo {
    /// No fixed size (not a DUP_FIXED table).
    None,
    /// Fixed size value with known key2 and value sizes.
    /// First element is key2 size, second is value size.
    /// Total stored size is key2_size + value_size.
    Size {
        /// Size of key2 in bytes.
        key2_size: usize,
        /// Size of value in bytes.
        value_size: usize,
    },
}

impl FixedSizeInfo {
    /// Returns true if the size info is known.
    pub const fn is_size(&self) -> bool {
        matches!(self, Self::Size { .. })
    }

    /// Returns true if there is no fixed size (not a DUP_FIXED table).
    pub const fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// Returns the total stored size (key2 + value) if known.
    pub const fn total_size(&self) -> Option<usize> {
        match self {
            Self::Size { key2_size, value_size } => Some(*key2_size + *value_size),
            _ => None,
        }
    }

    /// Returns the key2 size if known.
    pub const fn key2_size(&self) -> Option<usize> {
        match self {
            Self::Size { key2_size, .. } => Some(*key2_size),
            _ => None,
        }
    }

    /// Returns the value size if known.
    pub const fn value_size(&self) -> Option<usize> {
        match self {
            Self::Size { value_size, .. } => Some(*value_size),
            _ => None,
        }
    }
}

/// Information about an MDBX database.
pub struct DbInfo {
    flags: DatabaseFlags,
    dup_fixed_val_size: FixedSizeInfo,
    dbi: u32,
}

impl std::fmt::Debug for DbInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let flags = self.flags.iter_names().map(|t| t.0).collect::<Vec<_>>().join("|");

        f.debug_struct("DbInfo")
            .field("flags", &flags)
            .field("dbi", &self.dbi)
            .field("fixed_val_size", &self.dup_fixed_val_size)
            .finish()
    }
}

impl Clone for DbInfo {
    fn clone(&self) -> Self {
        Self {
            flags: DatabaseFlags::from_bits(self.flags.bits()).unwrap(),
            dbi: self.dbi,
            dup_fixed_val_size: self.dup_fixed_val_size,
        }
    }
}

impl DbInfo {
    /// Creates a new `DbInfo`.
    pub(crate) const fn new(
        flags: DatabaseFlags,
        dbi: u32,
        dup_fixed_val_size: FixedSizeInfo,
    ) -> Self {
        Self { flags, dbi, dup_fixed_val_size }
    }

    /// Returns the flags of the database.
    pub const fn flags(&self) -> DatabaseFlags {
        DatabaseFlags::from_bits(self.flags.bits()).unwrap()
    }

    /// Returns true if the database has the INTEGER_KEY flag.
    pub const fn is_integerkey(&self) -> bool {
        self.flags.contains(DatabaseFlags::INTEGER_KEY)
    }

    /// Returns true if the database has the DUP_SORT flag.
    pub const fn is_dupsort(&self) -> bool {
        self.flags.contains(DatabaseFlags::DUP_SORT)
    }

    /// Returns true if the database has the DUP_FIXED flag.
    pub const fn is_dupfixed(&self) -> bool {
        self.flags.contains(DatabaseFlags::DUP_FIXED)
    }

    /// Returns the fixed value size of the database, if any. This will be the
    /// size of the values in a DUP_FIXED database.
    ///
    /// This will be set the the SUM of the sizes of the key2 and value for
    /// dual-keyed tables.
    pub const fn dup_fixed_val_size(&self) -> FixedSizeInfo {
        self.dup_fixed_val_size
    }

    /// Returns the dbi of the database.
    pub const fn dbi(&self) -> u32 {
        self.dbi
    }
}

impl ValSer for FixedSizeInfo {
    fn encoded_size(&self) -> usize {
        2 * 4 // two u32 values
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        match self {
            FixedSizeInfo::None => {
                buf.put_u32(0);
                buf.put_u32(0);
            }
            FixedSizeInfo::Size { key2_size, value_size } => {
                buf.put_u32(*key2_size as u32);
                buf.put_u32(*value_size as u32);
            }
        }
    }

    fn decode_value(data: &[u8]) -> Result<Self, crate::hot::DeserError>
    where
        Self: Sized,
    {
        let key2_size = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
        let value_size = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
        if key2_size == 0 || value_size == 0 {
            Ok(FixedSizeInfo::None)
        } else {
            Ok(FixedSizeInfo::Size { key2_size, value_size })
        }
    }
}

impl ValSer for DbInfo {
    fn encoded_size(&self) -> usize {
        // 4 u32s
        4 + 4 + 8
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        self.dbi.encode_value_to(buf);
        self.flags.bits().encode_value_to(buf);
        self.dup_fixed_val_size.encode_value_to(buf);
    }

    fn decode_value(data: &[u8]) -> Result<Self, crate::hot::DeserError>
    where
        Self: Sized,
    {
        let dbi = u32::decode_value(&data[0..4])?;
        let flags_bits = u32::decode_value(&data[4..8])?;
        let flags = DatabaseFlags::from_bits(flags_bits).ok_or_else(|| {
            crate::hot::DeserError::String("Invalid database flags bits".to_string())
        })?;
        let dup_fixed_val_size = FixedSizeInfo::decode_value(&data[8..16])?;
        Ok(Self { flags, dbi, dup_fixed_val_size })
    }
}
