//! `std`-style collections whose nodes are linked by narrow [`rustlink`] links.
//!
//! Every container keeps its nodes in a `Vec` and refers to them by index through
//! [`rustlink::OptAbs`] links of a width you choose (`u16`, `u32` or `u64`), instead of through
//! 8-byte pointers and one heap allocation per node. The width bounds how many nodes a container
//! can hold: a `u16` container addresses at most 65,535 nodes. Operations that would need more
//! return [`Full`] from their `try_` form and panic in their plain form.
//!
//! | Container | Mirrors | Nodes |
//! |---|---|---|
//! | [`LinkedList`] | `std::collections::LinkedList` | one per element |
//! | [`BTreeMap`], [`BTreeSet`] | `std::collections::BTreeMap`, `BTreeSet` | B+tree leaves and inner nodes of fixed capacity |
//! | [`HashMap`], [`HashSet`] | `std::collections::HashMap`, `HashSet` | one entry per element, chained |
//!
//! ```
//! use rustlink_collections::LinkedList;
//!
//! // Two bytes per link: nodes of a `u32` element take 8 bytes instead of 24.
//! let mut list: LinkedList<u32, u16> = LinkedList::new();
//! list.push_back(2);
//! list.push_front(1);
//! assert_eq!(list.iter().copied().collect::<Vec<_>>(), [1, 2]);
//! ```
//!
//! The crate is `no_std` with `alloc` and contains no `unsafe` code. That has two consequences
//! for the API: elements are moved with `Default` placeholders where a slot must stay valid
//! (`BTreeMap` needs `K: Default`, `V: Default`), and there is no `iter_mut` on the containers
//! whose iteration order differs from their storage order (`LinkedList`, `BTreeMap`), because
//! handing out disjoint `&mut` references in that order needs `unsafe`.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate alloc;

mod error;
mod list;

pub use error::Full;
pub use list::LinkedList;
