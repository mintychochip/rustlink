//! Hand-written index baselines: a `Vec` of nodes with machine-word or 32-bit index links.

#![allow(clippy::unnecessary_cast)]

use super::{TreeBuild, TreeView};
use crate::plan::Plan;
use crate::workload::{Limit, Reach};

macro_rules! plain_store {
    ($(#[$doc:meta])* $store:ident, $node:ident, $link:ty) => {
        #[derive(Clone, Copy)]
        #[repr(C)]
        struct $node {
            payload: u32,
            a: $link,
            b: $link,
        }

        $(#[$doc])*
        pub struct $store {
            nodes: Vec<$node>,
            root: usize,
        }

        impl $store {
            const NIL: $link = <$link>::MAX;

            fn leaf(key: u32) -> $node {
                $node { payload: key, a: Self::NIL, b: Self::NIL }
            }

            fn child(link: $link) -> Option<usize> {
                (link != Self::NIL).then_some(link as usize)
            }

            /// Distances of every link, `child index - parent index`.
            pub fn reach(&self) -> Reach {
                Reach::from_deltas(self.nodes.iter().enumerate().flat_map(|(parent, node)| {
                    [node.a, node.b]
                        .into_iter()
                        .filter_map(Self::child)
                        .map(move |child| child as i64 - parent as i64)
                }))
            }
        }

        impl TreeView for $store {
            type Ref<'a> = usize;

            fn root(&self) -> Option<usize> {
                (!self.nodes.is_empty()).then_some(self.root)
            }

            fn payload(&self, r: usize) -> u32 {
                self.nodes[r].payload
            }

            fn a(&self, r: usize) -> Option<usize> {
                Self::child(self.nodes[r].a)
            }

            fn b(&self, r: usize) -> Option<usize> {
                Self::child(self.nodes[r].b)
            }
        }

        impl TreeBuild for $store {
            const NODE_BYTES: usize = size_of::<$node>();
            const LIMIT: Limit = Limit::None;

            fn empty(capacity: usize) -> Self {
                $store { nodes: Vec::with_capacity(capacity), root: 0 }
            }

            fn from_plan(plan: &Plan) -> Self {
                let link = |child: Option<u32>| child.map_or(Self::NIL, |i| i as $link);
                let nodes = plan
                    .nodes
                    .iter()
                    .map(|spec| $node { payload: spec.payload, a: link(spec.a), b: link(spec.b) })
                    .collect();
                $store { nodes, root: plan.root().unwrap_or(0) }
            }

            fn insert_bst(&mut self, key: u32) {
                if self.nodes.is_empty() {
                    self.nodes.push(Self::leaf(key));
                    self.root = 0;
                    return;
                }
                let mut at = self.root;
                loop {
                    let node = self.nodes[at];
                    if key == node.payload {
                        return;
                    }
                    let go_a = key < node.payload;
                    let next = if go_a { node.a } else { node.b };
                    if next == Self::NIL {
                        let new = self.nodes.len() as $link;
                        self.nodes.push(Self::leaf(key));
                        if go_a {
                            self.nodes[at].a = new;
                        } else {
                            self.nodes[at].b = new;
                        }
                        return;
                    }
                    at = next as usize;
                }
            }

            fn len(&self) -> usize {
                self.nodes.len()
            }
        }
    };
}

plain_store! {
    /// Nodes in a `Vec`, links are `usize` indices (24-byte nodes).
    PlainUsize, UsizeNode, usize
}

plain_store! {
    /// Nodes in a `Vec`, links are `u32` indices (12-byte nodes): what a Rust developer writes
    /// by hand, and the baseline `abs32` must match.
    PlainU32, U32Node, u32
}
