mod support;

use rustlink::{Abs, Arena, Dangling, OptAbs, Region, Rel};
use support::{NodeA32, NodeR16, arena_insert, in_order};

fn tree<N: support::BstNode>() -> Arena<N> {
    // Keys 5, 3, 8, 1, 4 land at indices 0, 1, 2, 3, 4.
    let mut arena = Arena::new();
    for key in [5, 3, 8, 1, 4] {
        arena_insert(&mut arena, key).unwrap();
    }
    arena
}

#[test]
fn len_get_and_is_empty() {
    let arena = tree::<NodeA32>();
    assert_eq!(arena.len(), 5);
    assert!(!arena.is_empty());
    assert_eq!(arena.get(2).unwrap().key.get(), 8);
    assert!(arena.get(5).is_none());
    assert!(Arena::<NodeA32>::new().is_empty());
}

#[test]
fn follow_resolves_absolute_links() {
    let arena = tree::<NodeA32>();
    let root = arena.get(0).unwrap();
    let (index, left) = arena.follow(0, root.left).unwrap();
    assert_eq!((index, left.key.get()), (1, 3));
    let (index, right) = arena.follow(0, root.right).unwrap();
    assert_eq!((index, right.key.get()), (2, 8));
}

#[test]
fn follow_is_none_for_null_and_for_out_of_range_targets() {
    let arena = tree::<NodeA32>();
    let leaf = arena.get(2).unwrap();
    assert!(arena.follow(2, leaf.left).is_none());
    let far = Abs::<NodeA32, u16>::new(99).unwrap();
    assert!(arena.follow(0, far).is_none());
}

#[test]
fn try_follow_distinguishes_null_from_dangling() {
    let arena = tree::<NodeA32>();
    let leaf = arena.get(2).unwrap();
    assert_eq!(
        arena.try_follow(2, leaf.left).map(|r| r.map(|(i, _)| i)),
        Ok(None)
    );
    let far = OptAbs::<NodeA32, u16>::new(Some(99)).unwrap();
    assert_eq!(
        arena.try_follow(3, far).map(|r| r.map(|(i, _)| i)),
        Err(Dangling { owner: 3 })
    );
}

#[test]
fn relative_links_resolve_from_the_owner() {
    let arena = tree::<NodeR16>();
    assert_eq!(in_order(&arena), vec![1, 3, 4, 5, 8]);

    // The same stored link means a different target for a different owner.
    let link = Rel::<NodeR16, u16>::new(3, 1).unwrap(); // delta -2
    assert_eq!(arena.follow(3, link).map(|(i, _)| i), Some(1));
    assert_eq!(arena.follow(4, link).map(|(i, _)| i), Some(2));
}

#[test]
fn relative_underflow_is_dangling_not_a_wrapped_index() {
    let mut arena: Arena<NodeR16> = Arena::new();
    arena_insert(&mut arena, 1).unwrap();
    let before_start = Rel::<NodeR16, u16>::new(5, 0).unwrap(); // delta -5
    assert!(arena.follow(0, before_start).is_none());
    assert_eq!(
        arena.try_follow(0, before_start).map(|r| r.map(|(i, _)| i)),
        Err(Dangling { owner: 0 })
    );
}

#[test]
fn cursor_requires_an_in_range_index() {
    let arena = tree::<NodeA32>();
    assert!(arena.cursor(4).is_some());
    assert!(arena.cursor(5).is_none());
    assert!(Arena::<NodeA32>::new().cursor(0).is_none());
}

#[test]
fn cursor_traversal_visits_keys_in_order() {
    assert_eq!(in_order(&tree::<NodeA32>()), vec![1, 3, 4, 5, 8]);
}

#[test]
fn cursor_follow_and_try_follow_use_the_cursor_as_owner() {
    let arena = tree::<NodeA32>();
    let root = arena.cursor(0).unwrap();
    assert_eq!(root.index(), 0);
    let left = root.follow(root.get().left).unwrap();
    assert_eq!((left.index(), left.get().key.get()), (1, 3));
    let leaf = arena.cursor(2).unwrap();
    assert!(matches!(leaf.try_follow(leaf.get().left), Ok(None)));
    let far = OptAbs::<NodeA32, u16>::new(Some(99)).unwrap();
    assert!(matches!(leaf.try_follow(far), Err(Dangling { owner: 2 })));
    assert!(leaf.follow(far).is_none());
}
