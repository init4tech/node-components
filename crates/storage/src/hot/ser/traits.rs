use crate::hot::ser::error::DeserError;
use alloy::primitives::Bytes;

/// Maximum allowed key size in bytes.
pub const MAX_KEY_SIZE: usize = 64;

/// The maximum size of a dual key (in bytes).
pub const MAX_FIXED_VAL_SIZE: usize = 64;

/// Trait for key serialization with fixed-size keys of size no greater than 32
/// bytes.
///
/// Keys must be FIXED SIZE, of size no greater than `MAX_KEY_SIZE` (64), and
/// no less than 1. The serialization must preserve ordering, i.e., for any two
/// keys `k1` and `k2`, if `k1 > k2`, then the byte representation of `k1`
/// must be lexicographically greater than that of `k2`.
///
/// In practice, keys are often hashes, addresses, numbers, or composites
/// of these.
pub trait KeySer: PartialOrd + Ord + Sized + Clone + core::fmt::Debug {
    /// The fixed size of the serialized key in bytes.
    /// Must satisfy `SIZE <= MAX_KEY_SIZE`.
    const SIZE: usize;

    /// Compile-time assertion to ensure SIZE is within limits.
    #[doc(hidden)]
    const ASSERT: sealed::Seal = {
        assert!(
            Self::SIZE <= MAX_KEY_SIZE,
            "KeySer implementations must have SIZE <= MAX_KEY_SIZE"
        );
        assert!(Self::SIZE > 0, "KeySer implementations must have SIZE > 0");
        sealed::Seal
    };

    /// Encode the key, optionally using the provided buffer.
    ///
    /// # Returns
    ///
    /// A slice containing the encoded key. This may be a slice of `buf`, or may
    /// be borrowed from the key itself. This slice must be <= `SIZE` bytes.
    fn encode_key<'a: 'c, 'b: 'c, 'c>(&'a self, buf: &'b mut [u8; MAX_KEY_SIZE]) -> &'c [u8];

    /// Decode a key from a byte slice.
    ///
    /// # Arguments
    /// * `data` - Exactly `SIZE` bytes to decode from.
    ///
    /// # Errors
    /// Returns an error if `data.len() != SIZE` or decoding fails.
    fn decode_key(data: &[u8]) -> Result<Self, DeserError>;

    /// Decode an optional key from an optional byte slice.
    ///
    /// Useful in DB decoding, where the absence of a key is represented by
    /// `None`.
    fn maybe_decode_key(data: Option<&[u8]>) -> Result<Option<Self>, DeserError> {
        match data {
            Some(d) => Ok(Some(Self::decode_key(d)?)),
            None => Ok(None),
        }
    }
}

/// Trait for value serialization.
///
/// Values can be of variable size, but must implement accurate size reporting.
/// When serialized, value sizes must be self-describing. I.e. the value must
/// tolerate being deserialized from a byte slice of arbitrary length, consuming
/// only as many bytes as needed.
///
/// E.g. a correct implementation for an array serializes the length of the
/// array first, so that the deserializer knows how many items to expect.
pub trait ValSer {
    /// The fixed size of the value, if applicable.
    const FIXED_SIZE: Option<usize> = None;

    /// The encoded size of the value in bytes. This MUST be accurate, as it is
    /// used to allocate buffers for serialization. Inaccurate sizes may result
    /// in panics or incorrect behavior.
    fn encoded_size(&self) -> usize;

    /// Serialize the value into bytes.
    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>;

    /// Serialize the value into bytes and return them.
    fn encoded(&self) -> Bytes {
        let mut buf = bytes::BytesMut::new();
        self.encode_value_to(&mut buf);
        buf.freeze().into()
    }

    /// Deserialize the value from bytes.
    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized;

    /// Deserialize an optional value from an optional byte slice.
    ///
    /// Useful in DB decoding, where the absence of a value is represented by
    /// `None`.
    fn maybe_decode_value(data: Option<&[u8]>) -> Result<Option<Self>, DeserError>
    where
        Self: Sized,
    {
        match data {
            Some(d) => Ok(Some(Self::decode_value(d)?)),
            None => Ok(None),
        }
    }

    /// Deserialize the value from bytes, ensuring all bytes are consumed.
    fn decode_value_exact(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let val = Self::decode_value(data)?;
        (val.encoded_size() == data.len())
            .then_some(val)
            .ok_or(DeserError::InexactDeser { extra_bytes: data.len() })
    }
}

mod sealed {
    /// Sealed struct to prevent overriding the `KeySer::ASSERT` constant.
    #[allow(
        dead_code,
        unreachable_pub,
        missing_copy_implementations,
        missing_debug_implementations
    )]
    pub struct Seal;
}
