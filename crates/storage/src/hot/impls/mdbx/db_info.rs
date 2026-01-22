use crate::hot::ValSer;
use reth_libmdbx::DatabaseFlags;

/// Type alias for the database info cache.
pub type DbCache = std::sync::Arc<dashmap::DashMap<&'static str, DbInfo>>;

/// Information about fixed size values in a database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixedSizeInfo {
    /// Not a DUPSORT table.
    None,
    /// DUPSORT table without DUP_FIXED (variable value size).
    DupSort {
        /// Size of key2 in bytes.
        key2_size: usize,
    },
    /// DUP_FIXED table with known key2 and total size.
    DupFixed {
        /// Size of key2 in bytes.
        key2_size: usize,
        /// Total fixed size (key2 + value).
        total_size: usize,
    },
}

impl FixedSizeInfo {
    /// Returns true if this is a DUP_FIXED table with known total size.
    pub const fn is_dup_fixed(&self) -> bool {
        matches!(self, Self::DupFixed { .. })
    }

    /// Returns true if there is no fixed size (not a DUPSORT table).
    pub const fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// Returns true if this is a DUPSORT table (with or without DUP_FIXED).
    pub const fn is_dupsort(&self) -> bool {
        matches!(self, Self::DupSort { .. } | Self::DupFixed { .. })
    }

    /// Returns the key2 size if known (for DUPSORT tables).
    pub const fn key2_size(&self) -> Option<usize> {
        match self {
            Self::DupSort { key2_size } => Some(*key2_size),
            Self::DupFixed { key2_size, .. } => Some(*key2_size),
            Self::None => None,
        }
    }

    /// Returns the total stored size (key2 + value) if known (only for DUP_FIXED tables).
    pub const fn total_size(&self) -> Option<usize> {
        match self {
            Self::DupFixed { total_size, .. } => Some(*total_size),
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
        8 // two u32s: key2_size and total_size
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
            FixedSizeInfo::DupSort { key2_size } => {
                buf.put_u32(*key2_size as u32);
                buf.put_u32(0); // total_size = 0 means variable value
            }
            FixedSizeInfo::DupFixed { key2_size, total_size } => {
                buf.put_u32(*key2_size as u32);
                buf.put_u32(*total_size as u32);
            }
        }
    }

    fn decode_value(data: &[u8]) -> Result<Self, crate::hot::DeserError>
    where
        Self: Sized,
    {
        let key2_size = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
        let total_size = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
        if key2_size == 0 {
            Ok(FixedSizeInfo::None)
        } else if total_size == 0 {
            Ok(FixedSizeInfo::DupSort { key2_size })
        } else {
            Ok(FixedSizeInfo::DupFixed { key2_size, total_size })
        }
    }
}

impl ValSer for DbInfo {
    fn encoded_size(&self) -> usize {
        // 4 u32s: dbi + flags + key2_size + total_size
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
