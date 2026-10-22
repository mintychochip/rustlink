//! Shared test fixtures: narrow-link BST node types for every width and link form, a boxed
//! reference BST, and generic helpers that work on any `Region`.
#![allow(dead_code)]

use proptest::prelude::ProptestConfig;
use rustlink::{Arena, Cursor, Link, LinkError, OptAbs, OptRel, Region, impl_links};
use zerocopy::byteorder::little_endian::U32;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned};

/// Proptest configuration that works under Miri (no failure-persistence files).
pub fn config() -> ProptestConfig {
    ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

/// A BST node whose child links can be set generically.
pub trait BstNode: Copy {
    type Child: Link<Self>;
    fn leaf(key: u32) -> Self;
    fn key(&self) -> u32;
    fn left(&self) -> Self::Child;
    fn right(&self) -> Self::Child;
    fn set_left(&mut self, owner: usize, target: usize) -> Result<(), LinkError>;
    fn set_right(&mut self, owner: usize, target: usize) -> Result<(), LinkError>;
}

macro_rules! bst_node {
    ($name:ident, $link:ident, $w:ty, $schema:expr, |$owner:ident, $target:ident| $make:expr) => {
        #[derive(FromBytes, IntoBytes, Immutable, KnownLayout, Unaligned, Clone, Copy)]
        #[repr(C)]
        pub struct $name {
            pub key: U32,
            pub left: $link<$name, $w>,
            pub right: $link<$name, $w>,
        }

        impl_links!($name, schema = $schema, links = [left, right]);

        impl BstNode for $name {
            type Child = $link<$name, $w>;

            fn leaf(key: u32) -> Self {
                Self {
                    key: U32::new(key),
                    left: $link::NULL,
                    right: $link::NULL,
                }
            }
            fn key(&self) -> u32 {
                self.key.get()
            }
            fn left(&self) -> Self::Child {
                self.left
            }
            fn right(&self) -> Self::Child {
                self.right
            }
            fn set_left(&mut self, $owner: usize, $target: usize) -> Result<(), LinkError> {
                self.left = $make?;
                Ok(())
            }
            fn set_right(&mut self, $owner: usize, $target: usize) -> Result<(), LinkError> {
                self.right = $make?;
                Ok(())
            }
        }
    };
}

bst_node!(NodeA32, OptAbs, u32, 0x5445_5354_0001, |_owner, target| {
    OptAbs::new(Some(target))
});
bst_node!(NodeA16, OptAbs, u16, 0x5445_5354_0002, |_owner, target| {
    OptAbs::new(Some(target))
});
bst_node!(NodeR32, OptRel, u32, 0x5445_5354_0003, |owner, target| {
    OptRel::new(owner, Some(target))
});
bst_node!(NodeR16, OptRel, u16, 0x5445_5354_0004, |owner, target| {
    OptRel::new(owner, Some(target))
});
bst_node!(NodeR8, OptRel, u8, 0x5445_5354_0005, |owner, target| {
    OptRel::new(owner, Some(target))
});

/// Inserts `key` into the BST stored in `arena` (root at index 0). Duplicates are ignored.
pub fn arena_insert<N: BstNode>(arena: &mut Arena<N>, key: u32) -> Result<(), LinkError> {
    if arena.is_empty() {
        arena.push(N::leaf(key));
        return Ok(());
    }
    let mut at = 0usize;
    loop {
        let (node_key, child) = {
            let node = &arena.as_slice()[at];
            (
                node.key(),
                if key < node.key() {
                    node.left()
                } else {
                    node.right()
                },
            )
        };
        if key == node_key {
            return Ok(());
        }
        match arena.follow(at, child) {
            Some((next, _)) => at = next,
            None => {
                let new = arena.push(N::leaf(key));
                let node = arena.get_mut(at).expect("owner exists");
                return if key < node_key {
                    node.set_left(at, new)
                } else {
                    node.set_right(at, new)
                };
            }
        }
    }
}

/// True if the BST rooted at index 0 of `region` contains `key`.
pub fn region_contains<N: BstNode, R: Region<N>>(region: &R, key: u32) -> bool {
    let Some(mut cursor) = region.cursor(0) else {
        return false;
    };
    loop {
        let node = cursor.get();
        if key == node.key() {
            return true;
        }
        let child = if key < node.key() {
            node.left()
        } else {
            node.right()
        };
        match cursor.follow(child) {
            Some(next) => cursor = next,
            None => return false,
        }
    }
}

/// Keys of the BST rooted at index 0 of `region`, in order.
pub fn in_order<N: BstNode, R: Region<N>>(region: &R) -> Vec<u32> {
    fn walk<N: BstNode>(cursor: Cursor<'_, N>, out: &mut Vec<u32>) {
        let node = cursor.get();
        if let Some(left) = cursor.follow(node.left()) {
            walk(left, out);
        }
        out.push(node.key());
        if let Some(right) = cursor.follow(node.right()) {
            walk(right, out);
        }
    }
    let mut out = Vec::new();
    if let Some(root) = region.cursor(0) {
        walk(root, &mut out);
    }
    out
}

/// A BST of exactly `n` distinct keys (n < 1009) inserted in a scrambled order.
pub fn build_tree<N: BstNode>(n: usize) -> Arena<N> {
    let mut arena = Arena::new();
    for i in 0..n {
        arena_insert(&mut arena, ((i * 7919) % 1009) as u32).expect("links fit");
    }
    arena
}

/// Boxed reference BST used as the oracle in differential tests.
#[derive(Default)]
pub struct BoxBst {
    root: Option<Box<BoxNode>>,
}

struct BoxNode {
    key: u32,
    left: Option<Box<BoxNode>>,
    right: Option<Box<BoxNode>>,
}

impl BoxBst {
    pub fn insert(&mut self, key: u32) {
        let mut slot = &mut self.root;
        while let Some(node) = slot {
            slot = match key.cmp(&node.key) {
                core::cmp::Ordering::Less => &mut node.left,
                core::cmp::Ordering::Greater => &mut node.right,
                core::cmp::Ordering::Equal => return,
            };
        }
        *slot = Some(Box::new(BoxNode {
            key,
            left: None,
            right: None,
        }));
    }

    pub fn contains(&self, key: u32) -> bool {
        let mut slot = &self.root;
        while let Some(node) = slot {
            match key.cmp(&node.key) {
                core::cmp::Ordering::Equal => return true,
                core::cmp::Ordering::Less => slot = &node.left,
                core::cmp::Ordering::Greater => slot = &node.right,
            }
        }
        false
    }
}
