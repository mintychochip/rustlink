//! The rustlink variants: an `Arena` of align-1 nodes with narrow, bounds-checked links.

use rustlink::{Arena, Cursor, OptAbs, OptRel, Region, Width};
use zerocopy::byteorder::little_endian::U32;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned};

use super::{TreeBuild, TreeView};
use crate::plan::Plan;
use crate::workload::Limit;

macro_rules! narrow_store {
    (
        $(#[$doc:meta])* $store:ident, $node:ident, $link:ident, $w:ty,
        $limit:expr, |$owner:ident, $child:ident| $make:expr
    ) => {
        /// One node: a little-endian payload and two narrow links, 1-byte aligned.
        #[derive(FromBytes, IntoBytes, Immutable, KnownLayout, Unaligned, Clone, Copy)]
        #[repr(C)]
        pub struct $node {
            payload: U32,
            a: $link<$node, $w>,
            b: $link<$node, $w>,
        }

        $(#[$doc])*
        pub struct $store {
            arena: Arena<$node>,
            root: usize,
        }

        impl $store {
            fn link($owner: usize, $child: Option<usize>) -> $link<$node, $w> {
                $make.expect("reach checked by the workload")
            }

            fn leaf(key: u32) -> $node {
                $node { payload: U32::new(key), a: $link::NULL, b: $link::NULL }
            }
        }

        impl TreeView for $store {
            type Ref<'a> = Cursor<'a, $node>;

            fn root(&self) -> Option<Cursor<'_, $node>> {
                self.arena.cursor(self.root)
            }

            fn payload(&self, r: Cursor<'_, $node>) -> u32 {
                r.get().payload.get()
            }

            fn a<'a>(&'a self, r: Cursor<'a, $node>) -> Option<Cursor<'a, $node>> {
                r.follow(r.get().a)
            }

            fn b<'a>(&'a self, r: Cursor<'a, $node>) -> Option<Cursor<'a, $node>> {
                r.follow(r.get().b)
            }
        }

        impl TreeBuild for $store {
            const NODE_BYTES: usize = size_of::<$node>();
            const LIMIT: Limit = $limit;

            fn empty(capacity: usize) -> Self {
                $store { arena: Arena::with_capacity(capacity), root: 0 }
            }

            fn from_plan(plan: &Plan) -> Self {
                let mut arena = Arena::with_capacity(plan.nodes.len());
                for (owner, spec) in plan.nodes.iter().enumerate() {
                    arena.push($node {
                        payload: U32::new(spec.payload),
                        a: Self::link(owner, spec.a.map(|i| i as usize)),
                        b: Self::link(owner, spec.b.map(|i| i as usize)),
                    });
                }
                $store { arena, root: plan.root().unwrap_or(0) }
            }

            fn insert_bst(&mut self, key: u32) {
                if self.arena.is_empty() {
                    self.arena.push(Self::leaf(key));
                    self.root = 0;
                    return;
                }
                let mut at = self.root;
                loop {
                    let node = self.arena.as_slice()[at];
                    let at_key = node.payload.get();
                    if key == at_key {
                        return;
                    }
                    let go_a = key < at_key;
                    let link = if go_a { node.a } else { node.b };
                    match self.arena.follow(at, link) {
                        Some((next, _)) => at = next,
                        None => {
                            let new = self.arena.push(Self::leaf(key));
                            let link = Self::link(at, Some(new));
                            let parent = self.arena.get_mut(at).expect("the owner exists");
                            if go_a {
                                parent.a = link;
                            } else {
                                parent.b = link;
                            }
                            return;
                        }
                    }
                }
            }

            fn len(&self) -> usize {
                self.arena.len()
            }
        }
    };
}

narrow_store! {
    /// `OptAbs<_, u32>` links (12-byte nodes).
    NarrowAbs32, AbsNode32, OptAbs, u32,
    Limit::Abs { max_elements: <u32 as Width>::MAX },
    |_owner, child| OptAbs::new(child)
}

narrow_store! {
    /// `OptAbs<_, u16>` links (8-byte nodes).
    NarrowAbs16, AbsNode16, OptAbs, u16,
    Limit::Abs { max_elements: <u16 as Width>::MAX },
    |_owner, child| OptAbs::new(child)
}

narrow_store! {
    /// `OptRel<_, u32>` links (12-byte nodes).
    NarrowRel32, RelNode32, OptRel, u32,
    Limit::Rel { max_delta: <u32 as Width>::SIGNED_MAX },
    |owner, child| OptRel::new(owner, child)
}

narrow_store! {
    /// `OptRel<_, u16>` links (8-byte nodes).
    NarrowRel16, RelNode16, OptRel, u16,
    Limit::Rel { max_delta: <u16 as Width>::SIGNED_MAX },
    |owner, child| OptRel::new(owner, child)
}

narrow_store! {
    /// `OptRel<_, u8>` links (6-byte nodes).
    NarrowRel8, RelNode8, OptRel, u8,
    Limit::Rel { max_delta: <u8 as Width>::SIGNED_MAX },
    |owner, child| OptRel::new(owner, child)
}
