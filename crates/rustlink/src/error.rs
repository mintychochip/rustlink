//! Error types.

use core::fmt;

/// A link could not be created.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LinkError {
    /// The value does not fit the link's width. `value` is the index (absolute links) or the
    /// delta (relative links); `min..=max` is the representable range.
    OutOfRange {
        /// The value that was requested.
        value: i128,
        /// Smallest representable value.
        min: i128,
        /// Largest representable value.
        max: i128,
    },
    /// An optional relative link was asked to point at its own owner, which would encode as null.
    NullCollision,
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LinkError::OutOfRange { value, min, max } => {
                write!(
                    f,
                    "link value {value} is outside the representable range {min}..={max}"
                )
            }
            LinkError::NullCollision => f.write_str(
                "an optional relative link cannot point at its own owner (delta 0 is null)",
            ),
        }
    }
}

impl core::error::Error for LinkError {}

/// A link is not null but its target lies outside the region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dangling {
    /// Index of the element that owns the link.
    pub owner: usize,
}

impl fmt::Display for Dangling {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "link owned by element {} points outside the region",
            self.owner
        )
    }
}

impl core::error::Error for Dangling {}

/// A blob failed verification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum VerifyError {
    /// Fewer bytes than the header.
    Truncated,
    /// The magic bytes do not match.
    BadMagic,
    /// Unsupported format version.
    Version {
        /// The version found in the header.
        found: u32,
    },
    /// The header's element size differs from `size_of::<T>()`.
    ElemSize {
        /// The element size found in the header.
        found: u32,
        /// The element size of the requested type.
        expected: u32,
    },
    /// The header's schema id differs from the type's schema id.
    Schema {
        /// The schema id found in the header.
        found: u64,
        /// The schema id of the requested type.
        expected: u64,
    },
    /// `count * elem_size` overflows.
    CountOverflow,
    /// The byte length differs from the length the header implies.
    Length {
        /// The length the header implies.
        expected: u64,
        /// The actual length.
        actual: usize,
    },
    /// An element holds a link whose target is outside the blob.
    BadLink {
        /// Index of the offending element.
        index: usize,
    },
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VerifyError::Truncated => f.write_str("blob is shorter than its header"),
            VerifyError::BadMagic => f.write_str("blob magic bytes do not match"),
            VerifyError::Version { found } => write!(f, "unsupported blob version {found}"),
            VerifyError::ElemSize { found, expected } => {
                write!(f, "blob element size is {found}, expected {expected}")
            }
            VerifyError::Schema { found, expected } => {
                write!(f, "blob schema id is {found:#x}, expected {expected:#x}")
            }
            VerifyError::CountOverflow => f.write_str("blob element count overflows"),
            VerifyError::Length { expected, actual } => {
                write!(f, "blob is {actual} bytes, header implies {expected}")
            }
            VerifyError::BadLink { index } => {
                write!(
                    f,
                    "element {index} holds a link that points outside the blob"
                )
            }
        }
    }
}

impl core::error::Error for VerifyError {}
