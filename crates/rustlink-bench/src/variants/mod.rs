//! Link-representation variants of the tree structure.
//!
//! Every variant stores the same logical tree (a payload and two optional children per node) and
//! implements [`TreeView`] for traversal and [`TreeBuild`] for construction, so one generic
//! algorithm runs unchanged on pointers, hand-written indices and narrow rustlink links.

mod boxed;
mod narrow;
mod plain;

pub use boxed::BoxTree;
pub use narrow::{NarrowAbs16, NarrowAbs32, NarrowRel8, NarrowRel16, NarrowRel32};
pub use plain::{PlainU32, PlainUsize};

use crate::plan::{CONST_MASK, OP_MUL, OP_NEG, Plan};
use crate::workload::Limit;

/// Read access to a tree through a cheap, copyable reference type.
pub trait TreeView {
    /// A reference to one node: a pointer, an index or a cursor.
    type Ref<'a>: Copy
    where
        Self: 'a;

    /// The root, if the tree is not empty.
    fn root(&self) -> Option<Self::Ref<'_>>;
    /// The payload of `r`.
    fn payload(&self, r: Self::Ref<'_>) -> u32;
    /// The first child of `r` (BST: smaller keys).
    fn a<'a>(&'a self, r: Self::Ref<'a>) -> Option<Self::Ref<'a>>;
    /// The second child of `r` (BST: larger keys).
    fn b<'a>(&'a self, r: Self::Ref<'a>) -> Option<Self::Ref<'a>>;
}

/// Construction of a tree in one link representation.
pub trait TreeBuild: TreeView + Sized {
    /// `size_of` one node in bytes.
    const NODE_BYTES: usize;
    /// What the representation can address.
    const LIMIT: Limit;

    /// An empty tree with room for `capacity` nodes.
    fn empty(capacity: usize) -> Self;
    /// Builds the plan's tree. Children precede parents; the root is the last node.
    fn from_plan(plan: &Plan) -> Self;
    /// Top-down unbalanced BST insertion: payload is the key, `a` holds smaller keys, `b` larger
    /// ones. Duplicates are ignored and the first insertion becomes the root.
    fn insert_bst(&mut self, key: u32);
    /// Number of nodes.
    fn len(&self) -> usize;
    /// True if the tree has no nodes.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Names of every variant, in report order.
pub const VARIANTS: &[&str] = &[
    "box", "usize", "u32", "abs32", "abs16", "rel32", "rel16", "rel8",
];

/// Binds the type alias `$v` to the variant called `$variant` and evaluates `$body`, or yields an
/// `Err(String)` for an unknown name.
macro_rules! with_variant {
    ($variant:expr, $v:ident => $body:expr) => {
        match $variant {
            "box" => {
                type $v = $crate::variants::BoxTree;
                $body
            }
            "usize" => {
                type $v = $crate::variants::PlainUsize;
                $body
            }
            "u32" => {
                type $v = $crate::variants::PlainU32;
                $body
            }
            "abs32" => {
                type $v = $crate::variants::NarrowAbs32;
                $body
            }
            "abs16" => {
                type $v = $crate::variants::NarrowAbs16;
                $body
            }
            "rel32" => {
                type $v = $crate::variants::NarrowRel32;
                $body
            }
            "rel16" => {
                type $v = $crate::variants::NarrowRel16;
                $body
            }
            "rel8" => {
                type $v = $crate::variants::NarrowRel8;
                $body
            }
            other => Err(format!("unknown variant `{other}`")),
        }
    };
}
pub(crate) use with_variant;

/// BST lookup.
pub fn contains<V: TreeView>(view: &V, key: u32) -> bool {
    let mut cursor = view.root();
    while let Some(node) = cursor {
        let payload = view.payload(node);
        if key == payload {
            return true;
        }
        cursor = if key < payload {
            view.a(node)
        } else {
            view.b(node)
        };
    }
    false
}

/// Evaluates an expression tree with explicit stacks (no recursion). Returns the value and the
/// number of nodes visited.
pub fn eval<V: TreeView>(view: &V) -> (u32, u64) {
    enum Step {
        Enter,
        Negate,
        Combine,
    }
    let Some(root) = view.root() else {
        return (0, 0);
    };
    let mut stack = vec![(root, Step::Enter)];
    let mut values: Vec<u32> = Vec::new();
    let mut visits = 0u64;
    while let Some((node, step)) = stack.pop() {
        match step {
            Step::Enter => {
                visits += 1;
                match (view.a(node), view.b(node)) {
                    (None, _) => values.push(view.payload(node) & CONST_MASK),
                    (Some(a), None) => {
                        stack.push((node, Step::Negate));
                        stack.push((a, Step::Enter));
                    }
                    (Some(a), Some(b)) => {
                        stack.push((node, Step::Combine));
                        stack.push((b, Step::Enter));
                        stack.push((a, Step::Enter));
                    }
                }
            }
            Step::Negate => {
                let x = values.pop().expect("operand");
                debug_assert_eq!(view.payload(node) >> 24, OP_NEG);
                values.push(x.wrapping_neg());
            }
            Step::Combine => {
                let y = values.pop().expect("second operand");
                let x = values.pop().expect("first operand");
                values.push(if view.payload(node) >> 24 == OP_MUL {
                    x.wrapping_mul(y)
                } else {
                    x.wrapping_add(y)
                });
            }
        }
    }
    (values.pop().unwrap_or(0), visits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{Spec, reference_eval};
    use crate::rng::Rng;

    /// Runs `check::<V>()` for every variant.
    fn for_each_variant(mut check: impl FnMut(&str, &dyn Fn(&Plan) -> Option<(u32, u64)>)) {
        for &name in VARIANTS {
            let run = |plan: &Plan| -> Option<(u32, u64)> {
                let result: Result<Option<(u32, u64)>, String> = with_variant!(name, V => {
                    if V::LIMIT.admits(plan.nodes.len(), &plan.reach()) {
                        Ok(Some(eval(&V::from_plan(plan))))
                    } else {
                        Ok(None)
                    }
                });
                result.unwrap()
            };
            check(name, &run);
        }
    }

    #[test]
    fn eval_matches_the_reference_on_every_variant_that_can_hold_the_plan() {
        for_each_variant(|name, run| {
            for n in [1, 2, 3, 50, 1000, 4096] {
                let plan = Plan::expression(n, 7);
                if let Some((value, visits)) = run(&plan) {
                    assert_eq!(value, reference_eval(&plan), "{name} at n = {n}");
                    assert_eq!(visits, plan.nodes.len() as u64, "{name} at n = {n}");
                } else {
                    assert!(
                        matches!(name, "abs16" | "rel16" | "rel8"),
                        "{name} must hold n = {n}"
                    );
                }
            }
        });
    }

    #[test]
    fn every_variant_holds_the_small_plans_and_the_wide_ones_hold_the_big_plan() {
        let big = Plan::expression(4096, 7);
        for name in ["box", "usize", "u32", "abs32", "rel32"] {
            let result: Result<bool, String> = with_variant!(name, V => {
                Ok(V::LIMIT.admits(big.nodes.len(), &big.reach()))
            });
            assert!(result.unwrap(), "{name}");
        }
    }

    fn bst_check<V: TreeBuild>(keys: &[u32], probes: &[u32]) {
        let mut tree = V::empty(keys.len());
        for &key in keys {
            tree.insert_bst(key);
        }
        let mut sorted = keys.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(tree.len(), sorted.len());
        for &probe in keys.iter().chain(probes) {
            assert_eq!(contains(&tree, probe), sorted.binary_search(&probe).is_ok());
        }
    }

    #[test]
    fn insert_and_contains_agree_with_a_sorted_vector_on_every_variant() {
        let mut rng = Rng::new(11);
        // Few distinct values so duplicates are common; 120 keys keep rel8 in reach.
        let keys: Vec<u32> = (0..120).map(|_| rng.below(90) as u32).collect();
        let probes: Vec<u32> = (0..200).map(|_| rng.below(120) as u32).collect();
        for &name in VARIANTS {
            let result: Result<(), String> = with_variant!(name, V => {
                bst_check::<V>(&keys, &probes);
                Ok(())
            });
            result.unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    #[test]
    fn insert_into_an_empty_tree_makes_the_root() {
        let mut tree = PlainU32::empty(0);
        assert!(tree.root().is_none());
        tree.insert_bst(5);
        assert_eq!(tree.payload(tree.root().unwrap()), 5);
        tree.insert_bst(5);
        assert_eq!(tree.len(), 1);
    }

    #[test]
    fn node_sizes_match_the_documented_layouts() {
        assert_eq!(BoxTree::NODE_BYTES, 24);
        assert_eq!(PlainUsize::NODE_BYTES, 24);
        assert_eq!(PlainU32::NODE_BYTES, 12);
        assert_eq!(NarrowAbs32::NODE_BYTES, 12);
        assert_eq!(NarrowAbs16::NODE_BYTES, 8);
        assert_eq!(NarrowRel32::NODE_BYTES, 12);
        assert_eq!(NarrowRel16::NODE_BYTES, 8);
        assert_eq!(NarrowRel8::NODE_BYTES, 6);
    }

    #[test]
    fn limits_match_the_link_widths() {
        assert_eq!(
            NarrowAbs16::LIMIT,
            Limit::Abs {
                max_elements: 65_535
            }
        );
        assert_eq!(NarrowRel8::LIMIT, Limit::Rel { max_delta: 127 });
        assert_eq!(NarrowRel16::LIMIT, Limit::Rel { max_delta: 32_767 });
        assert_eq!(BoxTree::LIMIT, Limit::None);
    }

    #[test]
    fn rel8_cannot_hold_a_balanced_tree_of_1024_but_u32_can() {
        let plan = Plan::balanced_bst(1024);
        assert!(!NarrowRel8::LIMIT.admits(plan.nodes.len(), &plan.reach()));
        assert!(NarrowRel32::LIMIT.admits(plan.nodes.len(), &plan.reach()));
        assert!(PlainU32::LIMIT.admits(plan.nodes.len(), &plan.reach()));
    }

    #[test]
    fn plain_reach_measures_every_link() {
        let plan = Plan::balanced_bst(1000);
        let tree = PlainU32::from_plan(&plan);
        assert_eq!(tree.reach(), plan.reach());
    }

    #[test]
    fn dropping_a_very_deep_boxed_tree_does_not_overflow_the_stack() {
        let nodes = (0..200_000u32)
            .map(|i| Spec {
                payload: OP_NEG << 24,
                a: i.checked_sub(1),
                b: None,
            })
            .collect();
        let tree = BoxTree::from_plan(&Plan { nodes });
        assert_eq!(tree.len(), 200_000);
        drop(tree);
    }

    #[test]
    fn eval_of_a_deep_chain_does_not_recurse() {
        let mut nodes: Vec<Spec> = vec![Spec {
            payload: 3,
            a: None,
            b: None,
        }];
        nodes.extend((1..100_000u32).map(|i| Spec {
            payload: OP_NEG << 24,
            a: Some(i - 1),
            b: None,
        }));
        let plan = Plan { nodes };
        let tree = PlainU32::from_plan(&plan);
        assert_eq!(eval(&tree), (reference_eval(&plan), 100_000));
    }

    #[test]
    fn an_unknown_variant_is_an_error() {
        let result: Result<(), String> = with_variant!("nope", V => {
            let _ = V::NODE_BYTES;
            Ok(())
        });
        assert_eq!(result, Err("unknown variant `nope`".to_owned()));
    }
}
