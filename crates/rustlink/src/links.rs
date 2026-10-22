//! The [`Links`] trait and the [`impl_links!`](crate::impl_links) macro.

/// Element types that can live in a blob.
///
/// Implement it with [`impl_links!`](crate::impl_links).
pub trait Links {
    /// A user-chosen layout identifier, stored in the blob header. Change it whenever the
    /// element layout changes.
    const SCHEMA: u64;

    /// True if every same-region link in `self` is null or in range for a region of `len`
    /// elements. `owner` is the index of `self`.
    fn check_links(&self, owner: usize, len: usize) -> bool;
}

/// Implements [`Links`] for a node type.
///
/// ```
/// use rustlink::{impl_links, OptAbs};
/// use zerocopy::byteorder::little_endian::U32;
/// use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned};
///
/// #[derive(FromBytes, IntoBytes, Immutable, KnownLayout, Unaligned)]
/// #[repr(C)]
/// struct Node {
///     key: U32,
///     left: OptAbs<Node, u32>,
///     right: OptAbs<Node, u32>,
/// }
///
/// impl_links!(Node, schema = 0x524C_4E4B_0001, links = [left, right]);
/// ```
///
/// List only links into the same region. Links into another region are not range-checked by
/// blob verification; they stay bounds-checked whenever they are followed.
#[macro_export]
macro_rules! impl_links {
    ($node:ty, schema = $schema:expr, links = [$($field:ident),* $(,)?]) => {
        impl $crate::Links for $node {
            const SCHEMA: u64 = $schema;

            fn check_links(&self, owner: usize, len: usize) -> bool {
                true $(&& $crate::Link::<$node>::check(self.$field, owner, len))*
            }
        }
    };
}
