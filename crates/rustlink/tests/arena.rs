mod support;

use proptest::prelude::*;
use rustlink::{Abs, Arena, LinkError, Region};
use support::{
    BoxBst, BstNode, NodeA16, NodeA32, NodeR8, NodeR16, NodeR32, arena_insert, region_contains,
};

#[test]
fn push_returns_sequential_indices() {
    let mut arena = Arena::with_capacity(4);
    assert_eq!(arena.push('a'), 0);
    assert_eq!(arena.push('b'), 1);
    assert_eq!(arena.push('c'), 2);
    assert_eq!(arena.len(), 3);
}

#[test]
fn get_mut_patches_a_link_after_construction() {
    let mut arena: Arena<NodeA32> = Arena::new();
    arena.push(NodeA32::leaf(1));
    arena.push(NodeA32::leaf(2));
    let root = arena.cursor(0).unwrap();
    assert!(root.follow(root.get().left).is_none());

    arena.get_mut(0).unwrap().set_left(0, 1).unwrap();

    let root = arena.cursor(0).unwrap();
    assert_eq!(root.follow(root.get().left).unwrap().index(), 1);
    assert!(arena.get_mut(2).is_none());
}

#[test]
fn clear_removes_every_element() {
    let mut arena = Arena::new();
    arena.push(1u8);
    arena.clear();
    assert!(arena.is_empty());
    assert!(arena.cursor(0).is_none());
}

#[test]
fn into_vec_keeps_insertion_order() {
    let mut arena = Arena::new();
    for value in [3u8, 1, 2] {
        arena.push(value);
    }
    assert_eq!(arena.into_vec(), vec![3, 1, 2]);
}

#[test]
fn debug_lists_the_elements() {
    let mut arena = Arena::new();
    arena.push(1u8);
    arena.push(2u8);
    assert_eq!(format!("{arena:?}"), "[1, 2]");
}

#[test]
fn links_into_another_region_resolve_there() {
    #[derive(Clone, Copy)]
    struct Vertex {
        id: u32,
    }
    #[derive(Clone, Copy)]
    struct Edge {
        to: Abs<Vertex, u16>,
    }

    let mut vertices = Arena::new();
    let a = vertices.push(Vertex { id: 10 });
    let mut edges = Arena::new();
    edges.push(Edge {
        to: Abs::new(a).unwrap(),
    });

    let edge = edges.get(0).unwrap();
    assert_eq!(vertices.follow(0, edge.to).unwrap().1.id, 10);
    let elsewhere: Arena<Vertex> = Arena::new();
    assert!(elsewhere.follow(0, edge.to).is_none());
}

macro_rules! differential {
    ($test:ident, $node:ty, $max_len:expr) => {
        proptest! {
            #![proptest_config(support::config())]

            #[test]
            fn $test(
                keys in proptest::collection::vec(0u32..1000, 0..$max_len),
                probes in proptest::collection::vec(0u32..1000, 0..64),
            ) {
                let mut reference = BoxBst::default();
                let mut arena: Arena<$node> = Arena::new();
                for &key in &keys {
                    reference.insert(key);
                    arena_insert(&mut arena, key).unwrap();
                }
                for &key in keys.iter().chain(probes.iter()) {
                    prop_assert_eq!(region_contains(&arena, key), reference.contains(key));
                }
            }
        }
    };
}

differential!(abs32_tree_matches_boxed_tree, NodeA32, 300);
differential!(abs16_tree_matches_boxed_tree, NodeA16, 300);
differential!(rel32_tree_matches_boxed_tree, NodeR32, 300);
differential!(rel16_tree_matches_boxed_tree, NodeR16, 300);
differential!(rel8_tree_matches_boxed_tree, NodeR8, 120);

#[test]
fn rel8_rejects_a_link_beyond_its_reach() {
    let mut arena: Arena<NodeR8> = Arena::new();
    arena_insert(&mut arena, 1000).unwrap(); // root, index 0
    // Keys 1..=200 chain down the root's left side; each new node hangs off the previous one
    // (delta 1), so every link fits in an i8.
    for key in 1..=200u32 {
        arena_insert(&mut arena, key).unwrap();
    }
    assert_eq!(arena.len(), 201);
    // The root's right side is still empty: this node lands at index 201, delta 201 > 127.
    assert!(matches!(
        arena_insert(&mut arena, 2000),
        Err(LinkError::OutOfRange { .. })
    ));
}
