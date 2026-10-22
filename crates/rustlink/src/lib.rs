#![doc = include_str!("../../../README.md")]
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
