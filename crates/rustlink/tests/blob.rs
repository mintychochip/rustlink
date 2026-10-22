mod support;

use rustlink::{Arena, BlobRef, HEADER_LEN, Region, VerifyError};
use support::{BstNode, NodeA16, NodeA32, NodeR8, NodeR16, NodeR32, build_tree, in_order};

#[test]
fn image_length_is_header_plus_elements() {
    let arena = build_tree::<NodeA32>(10);
    assert_eq!(arena.freeze().len(), HEADER_LEN + 10 * 12);
}

#[test]
fn freeze_into_appends_after_existing_bytes() {
    let arena = build_tree::<NodeA32>(3);
    let mut out = vec![0xEE; 5];
    arena.freeze_into(&mut out);
    assert_eq!(&out[..5], &[0xEE; 5]);
    assert!(BlobRef::<NodeA32>::verify(&out[5..]).is_ok());
}

macro_rules! round_trip {
    ($test:ident, $node:ty) => {
        #[test]
        fn $test() {
            let arena = build_tree::<$node>(100);
            let image = arena.freeze();
            let view = BlobRef::<$node>::verify(&image).unwrap();
            assert_eq!(view.len(), 100);
            assert_eq!(in_order(&view), in_order(&arena));
            for key in 0..1009 {
                assert_eq!(
                    support::region_contains(&view, key),
                    support::region_contains(&arena, key)
                );
            }
        }
    };
}

round_trip!(abs32_blob_round_trips, NodeA32);
round_trip!(abs16_blob_round_trips, NodeA16);
round_trip!(rel32_blob_round_trips, NodeR32);
round_trip!(rel16_blob_round_trips, NodeR16);
round_trip!(rel8_blob_round_trips, NodeR8);

#[test]
fn empty_blob_round_trips() {
    let arena: Arena<NodeA32> = Arena::new();
    let image = arena.freeze();
    assert_eq!(image.len(), HEADER_LEN);
    let view = BlobRef::<NodeA32>::verify(&image).unwrap();
    assert!(view.is_empty());
}

#[test]
fn view_is_independent_of_where_the_bytes_live() {
    let arena = build_tree::<NodeR16>(50);
    let image = arena.freeze();
    // Copy the image to an odd offset inside a larger buffer: no alignment requirement.
    let mut shifted = vec![0u8; 3];
    shifted.extend_from_slice(&image);
    let view = BlobRef::<NodeR16>::verify(&shifted[3..]).unwrap();
    assert_eq!(in_order(&view), in_order(&arena));
}

#[test]
fn rejects_images_shorter_than_the_header() {
    assert!(matches!(
        BlobRef::<NodeA32>::verify(&[0u8; 31]),
        Err(VerifyError::Truncated)
    ));
    assert!(matches!(
        BlobRef::<NodeA32>::verify(&[]),
        Err(VerifyError::Truncated)
    ));
}

#[test]
fn rejects_a_wrong_magic() {
    let mut image = build_tree::<NodeA32>(3).freeze();
    image[0] ^= 0xFF;
    assert!(matches!(
        BlobRef::<NodeA32>::verify(&image),
        Err(VerifyError::BadMagic)
    ));
}

#[test]
fn rejects_an_unknown_version() {
    let mut image = build_tree::<NodeA32>(3).freeze();
    image[8..12].copy_from_slice(&2u32.to_le_bytes());
    assert!(matches!(
        BlobRef::<NodeA32>::verify(&image),
        Err(VerifyError::Version { found: 2 })
    ));
}

#[test]
fn rejects_a_different_element_size() {
    let image = build_tree::<NodeA32>(3).freeze();
    assert!(matches!(
        BlobRef::<NodeA16>::verify(&image),
        Err(VerifyError::ElemSize {
            found: 12,
            expected: 8
        })
    ));
}

#[test]
fn rejects_a_different_schema_of_the_same_size() {
    let image = build_tree::<NodeA32>(3).freeze();
    assert!(matches!(
        BlobRef::<NodeR32>::verify(&image),
        Err(VerifyError::Schema {
            found: 0x5445_5354_0001,
            expected: 0x5445_5354_0003
        })
    ));
}

#[test]
fn rejects_a_count_that_overflows() {
    let mut image = build_tree::<NodeA32>(3).freeze();
    image[24..32].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(matches!(
        BlobRef::<NodeA32>::verify(&image),
        Err(VerifyError::CountOverflow)
    ));
}

#[test]
fn rejects_missing_and_trailing_bytes() {
    let image = build_tree::<NodeA32>(3).freeze();
    let expected = image.len() as u64;
    assert!(matches!(
        BlobRef::<NodeA32>::verify(&image[..image.len() - 1]),
        Err(VerifyError::Length { expected: e, actual }) if e == expected && actual == image.len() - 1
    ));
    let mut longer = image.clone();
    longer.push(0);
    assert!(matches!(
        BlobRef::<NodeA32>::verify(&longer),
        Err(VerifyError::Length { expected: e, actual }) if e == expected && actual == image.len() + 1
    ));
}

#[test]
fn rejects_a_link_that_points_outside_the_blob() {
    let mut image = build_tree::<NodeA32>(3).freeze();
    // Node 0 starts at HEADER_LEN: key (4 bytes), then the left link (4 bytes).
    image[HEADER_LEN + 4..HEADER_LEN + 8].copy_from_slice(&200u32.to_le_bytes());
    assert!(matches!(
        BlobRef::<NodeA32>::verify(&image),
        Err(VerifyError::BadLink { index: 0 })
    ));
}

/// Flips bytes of an image and checks the safety property: either `verify` rejects it, or every
/// link of the returned view resolves without dangling.
fn assert_tamper_safe<
    N: BstNode
        + rustlink::Links
        + zerocopy::FromBytes
        + zerocopy::KnownLayout
        + zerocopy::Immutable
        + zerocopy::Unaligned,
>(
    image: &[u8],
    values: impl Fn(u8) -> Vec<u8>,
) {
    for offset in 0..image.len() {
        for value in values(image[offset]) {
            let mut tampered = image.to_vec();
            tampered[offset] = value;
            if let Ok(view) = BlobRef::<N>::verify(&tampered) {
                for (index, node) in view.as_slice().iter().enumerate() {
                    assert!(view.try_follow(index, node.left()).is_ok());
                    assert!(view.try_follow(index, node.right()).is_ok());
                }
            }
        }
    }
}

#[test]
fn sampled_tampering_never_yields_a_dangling_view() {
    let image = build_tree::<NodeA32>(16).freeze();
    assert_tamper_safe::<NodeA32>(&image, |old| vec![0x00, 0x01, 0x7F, 0x80, 0xFF, old ^ 0x55]);
    let image = build_tree::<NodeR8>(16).freeze();
    assert_tamper_safe::<NodeR8>(&image, |old| vec![0x00, 0x01, 0x7F, 0x80, 0xFF, old ^ 0x55]);
}

#[test]
#[cfg_attr(miri, ignore = "exhaustive tampering is too slow under Miri")]
fn exhaustive_tampering_never_yields_a_dangling_view() {
    let image = build_tree::<NodeA32>(64).freeze();
    assert_tamper_safe::<NodeA32>(&image, |old| (0..=255u8).filter(|v| *v != old).collect());
    let image = build_tree::<NodeR16>(64).freeze();
    assert_tamper_safe::<NodeR16>(&image, |old| (0..=255u8).filter(|v| *v != old).collect());
}
