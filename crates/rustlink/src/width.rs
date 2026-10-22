//! Storage widths for links.

use zerocopy::byteorder::little_endian::{U16, U32, U64};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned};

mod sealed {
    pub trait Sealed {}
}

/// Unsigned width of a stored link offset.
///
/// Sealed: implemented for `u8`, `u16`, `u32` and `u64` only.
pub trait Width: sealed::Sealed + Copy + 'static {
    /// Storage size in bytes.
    const BYTES: usize;
    /// Largest unsigned value, `2^(8 * BYTES) - 1`.
    const MAX: u64;
    /// Smallest signed value, `-2^(8 * BYTES - 1)`.
    const SIGNED_MIN: i64;
    /// Largest signed value, `2^(8 * BYTES - 1) - 1`.
    const SIGNED_MAX: i64;
    /// The all-zero storage value.
    const ZERO: Self::Le;
    /// Alignment-1 little-endian storage for one offset.
    type Le: Copy
        + Eq
        + core::hash::Hash
        + core::fmt::Debug
        + FromBytes
        + IntoBytes
        + Immutable
        + KnownLayout
        + Unaligned;
    /// Encodes the low `BYTES` bytes of `raw`. The caller guarantees `raw <= MAX`.
    fn encode(raw: u64) -> Self::Le;
    /// Decodes the stored value as an unsigned integer.
    fn decode(le: Self::Le) -> u64;
    /// Encodes `v` as two's complement. The caller guarantees `SIGNED_MIN <= v <= SIGNED_MAX`.
    fn encode_signed(v: i64) -> Self::Le;
    /// Decodes the stored value as a sign-extended two's-complement integer.
    fn decode_signed(le: Self::Le) -> i64;
}

impl sealed::Sealed for u8 {}

impl Width for u8 {
    const BYTES: usize = 1;
    const MAX: u64 = u8::MAX as u64;
    const SIGNED_MIN: i64 = i8::MIN as i64;
    const SIGNED_MAX: i64 = i8::MAX as i64;
    const ZERO: u8 = 0;
    type Le = u8;

    fn encode(raw: u64) -> u8 {
        raw as u8
    }
    fn decode(le: u8) -> u64 {
        u64::from(le)
    }
    fn encode_signed(v: i64) -> u8 {
        v as i8 as u8
    }
    fn decode_signed(le: u8) -> i64 {
        i64::from(le as i8)
    }
}

macro_rules! impl_width_le {
    ($t:ty, $le:ident, $bytes:expr, $s:ty) => {
        impl sealed::Sealed for $t {}

        #[allow(clippy::unnecessary_cast)]
        impl Width for $t {
            const BYTES: usize = $bytes;
            const MAX: u64 = <$t>::MAX as u64;
            const SIGNED_MIN: i64 = <$s>::MIN as i64;
            const SIGNED_MAX: i64 = <$s>::MAX as i64;
            const ZERO: $le = $le::from_bytes([0; $bytes]);
            type Le = $le;

            fn encode(raw: u64) -> $le {
                $le::new(raw as $t)
            }
            fn decode(le: $le) -> u64 {
                u64::from(le.get())
            }
            fn encode_signed(v: i64) -> $le {
                $le::new(v as $s as $t)
            }
            fn decode_signed(le: $le) -> i64 {
                i64::from(le.get() as $s)
            }
        }
    };
}

impl_width_le!(u16, U16, 2, i16);
impl_width_le!(u32, U32, 4, i32);
impl_width_le!(u64, U64, 8, i64);
