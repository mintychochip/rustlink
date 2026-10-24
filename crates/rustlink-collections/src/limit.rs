//! The node capacity implied by a link width.

use rustlink::Width;

/// The most nodes a container with `W`-wide optional links can hold: the stored value is the
/// index plus one, so indices `0..W::MAX` are representable.
pub(crate) fn node_limit<W: Width>() -> usize {
    usize::try_from(W::MAX).unwrap_or(usize::MAX)
}
