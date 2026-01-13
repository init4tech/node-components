use crate::ser::error::DeserError;
use alloy::primitives::Bytes;

/// Maximum allowed key size in bytes.
pub const MAX_KEY_SIZE: usize = 64;

/// Trait for key serialization with fixed-size keys of size no greater than 32
/// bytes.
pub trait KeySer: Ord + Sized {
    /// The fixed size of the serialized key in bytes.
    /// Must satisfy `SIZE <= MAX_KEY_SIZE`.
    const SIZE: usize;

    /// Compile-time assertion to ensure SIZE is within limits.
    #[doc(hidden)]
    const ASSERT: () = {
        assert!(
            Self::SIZE <= MAX_KEY_SIZE,
            "KeySer implementations must have SIZE <= MAX_KEY_SIZE"
        );
        assert!(Self::SIZE > 0, "KeySer implementations must have SIZE > 0");
    };

    /// Encode the key into the provided buffer.
    ///
    /// Writes exactly `SIZE` bytes to `buf[..SIZE]` and returns `SIZE`.
    /// The encoding must preserve ordering: for any `k1, k2` where `k1 > k2`,
    /// the bytes written by `k1` must be lexicographically greater than those
    /// of `k2`.
    ///
    /// # Returns
    ///
    /// A slice containing the encoded key. This may be a slice of buf, or may
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
pub trait ValSer {
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

    /// Deserialize the value from bytes, advancing the `data` slice.
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
    fn decode_value_exact(data: &mut &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let val = Self::decode_value(data)?;
        data.is_empty().then_some(val).ok_or(DeserError::InexactDeser { extra_bytes: data.len() })
    }
}
