//! Frozen, verified, position-independent blobs.

use alloc::vec::Vec;

use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned};

use crate::{Arena, Links, Region, VerifyError};

/// Length of the blob header in bytes.
pub const HEADER_LEN: usize = 32;

const MAGIC: [u8; 8] = *b"RLNKBLOB";
const VERSION: u32 = 1;

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn read_u64(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes([
        bytes[at],
        bytes[at + 1],
        bytes[at + 2],
        bytes[at + 3],
        bytes[at + 4],
        bytes[at + 5],
        bytes[at + 6],
        bytes[at + 7],
    ])
}

impl<T> Arena<T>
where
    T: IntoBytes + Immutable + Unaligned + Links,
{
    /// Appends the blob image of this arena (header plus raw elements) to `out`.
    ///
    /// The image holds no addresses, so it can be written to a file, memory-mapped or sent to
    /// another process and read back with [`BlobRef::verify`].
    pub fn freeze_into(&self, out: &mut Vec<u8>) {
        const { assert!(size_of::<T>() > 0 && size_of::<T>() <= u32::MAX as usize) };
        let items = self.as_slice();
        out.reserve(HEADER_LEN + core::mem::size_of_val(items));
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&(size_of::<T>() as u32).to_le_bytes());
        out.extend_from_slice(&T::SCHEMA.to_le_bytes());
        out.extend_from_slice(&(items.len() as u64).to_le_bytes());
        out.extend_from_slice(items.as_bytes());
    }

    /// The blob image of this arena.
    pub fn freeze(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.freeze_into(&mut out);
        out
    }
}

/// A verified, zero-copy view of a blob.
///
/// Created by [`BlobRef::verify`]. Resolve links through the [`Region`] trait.
pub struct BlobRef<'a, T> {
    items: &'a [T],
}

impl<'a, T> BlobRef<'a, T>
where
    T: FromBytes + KnownLayout + Immutable + Unaligned + Links,
{
    /// Checks `bytes` and returns a view of its elements.
    ///
    /// Verification is O(n) and allocation-free. It checks the header, the exact length, the
    /// element size and schema id, and that every same-region link of every element is null or
    /// in range. Memory safety never depends on it: links stay bounds-checked when followed.
    pub fn verify(bytes: &'a [u8]) -> Result<Self, VerifyError> {
        const { assert!(size_of::<T>() > 0 && size_of::<T>() <= u32::MAX as usize) };
        if bytes.len() < HEADER_LEN {
            return Err(VerifyError::Truncated);
        }
        if bytes[..8] != MAGIC {
            return Err(VerifyError::BadMagic);
        }
        let version = read_u32(bytes, 8);
        if version != VERSION {
            return Err(VerifyError::Version { found: version });
        }
        let elem_size = read_u32(bytes, 12);
        if elem_size as usize != size_of::<T>() {
            return Err(VerifyError::ElemSize {
                found: elem_size,
                expected: size_of::<T>() as u32,
            });
        }
        let schema = read_u64(bytes, 16);
        if schema != T::SCHEMA {
            return Err(VerifyError::Schema {
                found: schema,
                expected: T::SCHEMA,
            });
        }
        let count = read_u64(bytes, 24);
        let expected = count
            .checked_mul(u64::from(elem_size))
            .and_then(|body| body.checked_add(HEADER_LEN as u64))
            .ok_or(VerifyError::CountOverflow)?;
        let length_error = VerifyError::Length {
            expected,
            actual: bytes.len(),
        };
        if expected != bytes.len() as u64 {
            return Err(length_error);
        }
        let count = usize::try_from(count).map_err(|_| VerifyError::CountOverflow)?;
        let items = <[T]>::ref_from_bytes_with_elems(&bytes[HEADER_LEN..], count)
            .map_err(|_| length_error)?;
        for (index, element) in items.iter().enumerate() {
            if !element.check_links(index, items.len()) {
                return Err(VerifyError::BadLink { index });
            }
        }
        Ok(Self { items })
    }
}

impl<T> Region<T> for BlobRef<'_, T> {
    fn as_slice(&self) -> &[T] {
        self.items
    }
}
