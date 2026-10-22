//! Narrow typed links: 1, 2, 4 or 8 byte offsets that replace 8-byte pointers inside
//! pointer-dense data structures.
//!
//! A link is stored as a little-endian offset of a chosen [`Width`] and is resolved against a
//! [`Region`] (a slice of elements) with a bounds check. Two forms exist:
//!
//! * [`Abs`] / [`OptAbs`]: an element index. The width bounds the region size.
//! * [`Rel`] / [`OptRel`]: a signed element delta from the *owner*, the element that stores the
//!   link. The width bounds the link distance, so a region can be arbitrarily large when links
//!   are local.
//!
//! All link types are `Copy`, have alignment 1 and implement the `zerocopy` traits, so node
//! types that contain them can derive the same traits. An [`Arena`] of such nodes can be
//! frozen into a position-independent blob and read back with [`BlobRef::verify`].

#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate alloc;

mod arena;
mod blob;
mod error;
mod link;
mod links;
mod region;
mod width;

pub use arena::Arena;
pub use blob::{BlobRef, HEADER_LEN};
pub use error::{Dangling, LinkError, VerifyError};
pub use link::{Abs, Link, OptAbs, OptRel, Rel, Target};
pub use links::Links;
pub use region::{Cursor, Region};
pub use width::Width;
