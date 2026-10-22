mod support;

use std::mem::{align_of, size_of};

use proptest::prelude::*;
use rustlink::{Abs, Link, LinkError, OptAbs, OptRel, Rel, Target, Width};
use support::{NodeA16, NodeA32, NodeR8, NodeR16, NodeR32};
use zerocopy::FromBytes;

/// Indices around the edges of width `W`.
fn edge_indices<W: Width>() -> Vec<usize> {
    let max = W::MAX;
    [0, 1, max.saturating_sub(1), max, max.saturating_add(1)]
        .into_iter()
        .filter_map(|v| usize::try_from(v).ok())
        .collect()
}

fn check_abs<W: Width>() {
    for index in edge_indices::<W>() {
        match Abs::<(), W>::new(index) {
            Ok(link) => {
                assert!(index as u64 <= W::MAX);
                assert_eq!(link.index(), index);
                assert_eq!(link.target(7), Target::Index(index));
            }
            Err(LinkError::OutOfRange { value, min, max }) => {
                assert!(index as u64 > W::MAX);
                assert_eq!((value, min, max), (index as i128, 0, i128::from(W::MAX)));
            }
            Err(other) => panic!("unexpected error {other:?}"),
        }
    }
}

fn check_opt_abs<W: Width>() {
    assert_eq!(OptAbs::<(), W>::new(None).unwrap().get(), None);
    assert_eq!(OptAbs::<(), W>::NULL.get(), None);
    assert_eq!(OptAbs::<(), W>::default(), OptAbs::NULL);
    for index in edge_indices::<W>() {
        match OptAbs::<(), W>::new(Some(index)) {
            Ok(link) => {
                assert!((index as u64) < W::MAX);
                assert_eq!(link.get(), Some(index));
                assert_eq!(link.target(0), Target::Index(index));
            }
            Err(LinkError::OutOfRange { .. }) => assert!(index as u64 >= W::MAX),
            Err(other) => panic!("unexpected error {other:?}"),
        }
    }
    assert_eq!(OptAbs::<(), W>::NULL.target(0), Target::Null);
}

fn check_rel<W: Width>(owner: usize, target: usize) {
    let delta = target as i128 - owner as i128;
    let fits = delta >= i128::from(W::SIGNED_MIN) && delta <= i128::from(W::SIGNED_MAX);
    match Rel::<(), W>::new(owner, target) {
        Ok(link) => {
            assert!(fits);
            assert_eq!(i128::from(link.delta()), delta);
            assert_eq!(link.target(owner), Target::Index(target));
        }
        Err(LinkError::OutOfRange { value, .. }) => {
            assert!(!fits);
            assert_eq!(value, delta);
        }
        Err(other) => panic!("unexpected error {other:?}"),
    }
}

fn check_opt_rel<W: Width>(owner: usize, target: usize) {
    if owner == target {
        assert_eq!(
            OptRel::<(), W>::new(owner, Some(target)),
            Err(LinkError::NullCollision)
        );
        return;
    }
    let delta = target as i128 - owner as i128;
    let fits = delta >= i128::from(W::SIGNED_MIN) && delta <= i128::from(W::SIGNED_MAX);
    match OptRel::<(), W>::new(owner, Some(target)) {
        Ok(link) => {
            assert!(fits);
            assert_eq!(link.delta().map(i128::from), Some(delta));
            assert_eq!(link.target(owner), Target::Index(target));
        }
        Err(LinkError::OutOfRange { .. }) => assert!(!fits),
        Err(other) => panic!("unexpected error {other:?}"),
    }
}

#[test]
fn abs_edges_at_every_width() {
    check_abs::<u8>();
    check_abs::<u16>();
    check_abs::<u32>();
    check_abs::<u64>();
}

#[test]
fn opt_abs_edges_at_every_width() {
    check_opt_abs::<u8>();
    check_opt_abs::<u16>();
    check_opt_abs::<u32>();
    check_opt_abs::<u64>();
}

#[test]
fn reach_matches_the_documented_table() {
    assert!(Abs::<(), u8>::new(255).is_ok());
    assert!(Abs::<(), u8>::new(256).is_err());
    assert!(OptAbs::<(), u8>::new(Some(254)).is_ok());
    assert!(OptAbs::<(), u8>::new(Some(255)).is_err());
    assert!(Rel::<(), u8>::new(200, 72).is_ok()); // delta -128
    assert!(Rel::<(), u8>::new(200, 71).is_err()); // delta -129
    assert!(Rel::<(), u8>::new(0, 127).is_ok());
    assert!(Rel::<(), u8>::new(0, 128).is_err());
    assert!(Rel::<(), u16>::new(0, 32_767).is_ok());
    assert!(Rel::<(), u16>::new(0, 32_768).is_err());
}

proptest! {
    #![proptest_config(support::config())]

    #[test]
    fn rel_round_trips_or_rejects(owner in 50_000usize..60_000, delta in -40_000i64..40_000) {
        let target = (owner as i64 + delta) as usize;
        check_rel::<u8>(owner, target);
        check_rel::<u16>(owner, target);
        check_rel::<u32>(owner, target);
        check_rel::<u64>(owner, target);
    }

    #[test]
    fn opt_rel_round_trips_or_rejects(owner in 50_000usize..60_000, delta in -300i64..300) {
        let target = (owner as i64 + delta) as usize;
        check_opt_rel::<u8>(owner, target);
        check_opt_rel::<u16>(owner, target);
        check_opt_rel::<u32>(owner, target);
        check_opt_rel::<u64>(owner, target);
    }

    #[test]
    fn opt_rel_null_never_collides_with_a_target(owner in any::<usize>()) {
        let null = OptRel::<(), u16>::new(owner, None).unwrap();
        prop_assert_eq!(null.delta(), None);
        prop_assert_eq!(null.target(owner), Target::Null);
    }

    #[test]
    fn decoding_arbitrary_bytes_never_panics(raw in any::<[u8; 8]>(), owner in any::<usize>()) {
        let r8 = Rel::<(), u8>::read_from_bytes(&raw[..1]).unwrap();
        let r16 = Rel::<(), u16>::read_from_bytes(&raw[..2]).unwrap();
        let r32 = Rel::<(), u32>::read_from_bytes(&raw[..4]).unwrap();
        let r64 = Rel::<(), u64>::read_from_bytes(&raw[..8]).unwrap();
        let o8 = OptRel::<(), u8>::read_from_bytes(&raw[..1]).unwrap();
        let o64 = OptRel::<(), u64>::read_from_bytes(&raw[..8]).unwrap();
        let a16 = Abs::<(), u16>::read_from_bytes(&raw[..2]).unwrap();
        let a64 = Abs::<(), u64>::read_from_bytes(&raw[..8]).unwrap();
        let p64 = OptAbs::<(), u64>::read_from_bytes(&raw[..8]).unwrap();
        for target in [
            r8.target(owner), r16.target(owner), r32.target(owner), r64.target(owner),
            o8.target(owner), o64.target(owner), a16.target(owner), a64.target(owner),
            p64.target(owner),
        ] {
            let _ = target;
        }
    }
}

#[test]
fn relative_underflow_is_invalid_and_fails_checks() {
    let link = Rel::<(), u16>::new(5, 0).unwrap(); // delta -5
    assert_eq!(link.target(2), Target::Invalid);
    assert!(!link.check(2, 10));
    assert_eq!(link.target(5), Target::Index(0));
    assert!(link.check(5, 1));
    assert!(!link.check(5, 0));
}

#[test]
fn check_accepts_null_and_in_range_only() {
    let null = OptAbs::<(), u16>::NULL;
    assert!(null.check(0, 0));
    let to_three = Abs::<(), u16>::new(3).unwrap();
    assert!(to_three.check(0, 4));
    assert!(!to_three.check(0, 3));
}

#[test]
fn links_have_the_width_as_size_and_alignment_one() {
    assert_eq!(
        (size_of::<Abs<(), u8>>(), align_of::<Abs<(), u8>>()),
        (1, 1)
    );
    assert_eq!(
        (size_of::<OptAbs<(), u16>>(), align_of::<OptAbs<(), u16>>()),
        (2, 1)
    );
    assert_eq!(
        (size_of::<Rel<(), u32>>(), align_of::<Rel<(), u32>>()),
        (4, 1)
    );
    assert_eq!(
        (size_of::<OptRel<(), u64>>(), align_of::<OptRel<(), u64>>()),
        (8, 1)
    );
}

#[test]
fn nodes_with_narrow_links_pack_to_the_expected_sizes() {
    assert_eq!(size_of::<NodeA32>(), 12);
    assert_eq!(size_of::<NodeR32>(), 12);
    assert_eq!(size_of::<NodeA16>(), 8);
    assert_eq!(size_of::<NodeR16>(), 8);
    assert_eq!(size_of::<NodeR8>(), 6);
    assert_eq!(align_of::<NodeR8>(), 1);
}

#[test]
fn links_ignore_their_element_type_for_equality_and_hashing() {
    struct NotHashable;
    let a = Abs::<NotHashable, u32>::new(9).unwrap();
    let b = Abs::<NotHashable, u32>::new(9).unwrap();
    assert_eq!(a, b);
    let mut set = std::collections::HashSet::new();
    set.insert(a);
    assert!(set.contains(&b));
}

#[test]
fn debug_output_shows_the_decoded_value() {
    assert_eq!(format!("{:?}", Abs::<(), u8>::new(3).unwrap()), "Abs(3)");
    assert_eq!(format!("{:?}", OptAbs::<(), u8>::NULL), "OptAbs(None)");
    assert_eq!(
        format!("{:?}", Rel::<(), u8>::new(5, 2).unwrap()),
        "Rel(-3)"
    );
    assert_eq!(
        format!("{:?}", OptRel::<(), u8>::new(5, Some(7)).unwrap()),
        "OptRel(Some(2))"
    );
}
