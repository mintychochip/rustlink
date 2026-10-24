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
//! | [`BTreeMap`], [`BTreeSet`] | `std::collections::BTreeMap`, `BTreeSet` | B+tree with Vec-backed 128-entry leaves and inner nodes |
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
//! The crate is `no_std` with `alloc` and contains no `unsafe` code. [`HashMap`] and [`HashSet`]
//! move keys and values out of removed slots with `Default` placeholders (`K: Default`,
//! `V: Default`), while [`BTreeMap`] only needs `K: Clone` to copy keys into inner-node
//! separators. [`HashMap`] exposes `iter_mut`; the containers whose iteration order differs from
//! storage order ([`BTreeMap`], [`LinkedList`]) provide `for_each_mut`, because handing out
//! disjoint `&mut` references in that order would need `unsafe`.
//!
//! ```
//! use rustlink_collections::{BTreeMap, HashMap};
//!
//! let mut ordered: BTreeMap<u32, u32, u16> = BTreeMap::new();
//! ordered.insert(2, 20);
//! ordered.insert(1, 10);
//! assert_eq!(ordered.iter().map(|(k, _)| *k).collect::<Vec<_>>(), [1, 2]);
//!
//! let mut hashed: HashMap<u32, u32, u16> = HashMap::new();
//! hashed.insert(2, 20);
//! assert_eq!(hashed.get(&2), Some(&20));
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate alloc;

mod btree;
mod error;
mod hash;
mod limit;
mod list;

pub use btree::{BTreeMap, BTreeSet, Range};
pub use error::Full;
pub use hash::{FnvBuildHasher, FnvHasher, HashMap, HashSet};
pub use list::LinkedList;
