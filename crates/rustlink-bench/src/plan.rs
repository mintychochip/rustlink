//! Tree plans: link-representation-independent descriptions of the trees the workloads build.

use crate::rng::Rng;
use crate::workload::Reach;

/// Expression operation: a leaf constant.
pub const OP_LEAF: u32 = 0;
/// Expression operation: wrapping addition of both operands.
pub const OP_ADD: u32 = 1;
/// Expression operation: wrapping negation of the first operand.
pub const OP_NEG: u32 = 2;
/// Expression operation: wrapping multiplication of both operands.
pub const OP_MUL: u32 = 3;
/// Mask of the 24-bit constant stored in a leaf payload.
pub const CONST_MASK: u32 = 0xFF_FFFF;

/// One node of a plan: a payload and its children as indices of earlier nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spec {
    /// BST: the key. Expression tree: the operation in the high 8 bits and, for leaves, a 24-bit
    /// constant in the low bits.
    pub payload: u32,
    /// First child (BST: smaller keys; expression: first operand).
    pub a: Option<u32>,
    /// Second child (BST: larger keys; expression: second operand).
    pub b: Option<u32>,
}

/// A tree described independently of any link representation.
///
/// Every child index is smaller than its parent's index and the root is the last node, so
/// creating the nodes in order always has the children available before their parents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// Nodes in creation order.
    pub nodes: Vec<Spec>,
}

impl Plan {
    /// Index of the root (the last node), or `None` for an empty plan.
    pub fn root(&self) -> Option<usize> {
        self.nodes.len().checked_sub(1)
    }

    /// Distances of every link, `child index - parent index`.
    pub fn reach(&self) -> Reach {
        Reach::from_deltas(self.nodes.iter().enumerate().flat_map(|(parent, spec)| {
            [spec.a, spec.b]
                .into_iter()
                .flatten()
                .map(move |child| i64::from(child) - parent as i64)
        }))
    }

    /// A perfectly balanced BST over the keys `0..n`: payload is the key, `a` the smaller side.
    /// Children are created before parents (post-order).
    pub fn balanced_bst(n: usize) -> Plan {
        fn build(nodes: &mut Vec<Spec>, lo: usize, hi: usize) -> Option<u32> {
            if lo >= hi {
                return None;
            }
            let mid = lo + (hi - lo) / 2;
            let a = build(nodes, lo, mid);
            let b = build(nodes, mid + 1, hi);
            nodes.push(Spec {
                payload: mid as u32,
                a,
                b,
            });
            Some((nodes.len() - 1) as u32)
        }
        let mut nodes = Vec::with_capacity(n);
        build(&mut nodes, 0, n);
        Plan { nodes }
    }

    /// A random expression tree of at least `n` nodes (`n >= 1`), built bottom-up with a stack of
    /// unfinished subtrees, so operands are always created before their operation.
    pub fn expression(n: usize, seed: u64) -> Plan {
        assert!(n >= 1, "an expression needs at least one node");
        let mut rng = Rng::new(seed);
        let mut nodes: Vec<Spec> = Vec::with_capacity(n + 64);
        let mut stack: Vec<u32> = Vec::new();
        while nodes.len() < n {
            let choice = rng.below(10);
            let index = nodes.len() as u32;
            if stack.len() >= 2 && choice < 4 {
                let (b, a) = (stack.pop(), stack.pop());
                let op = 1 + 2 * rng.below(2) as u32; // OP_ADD or OP_MUL
                nodes.push(Spec {
                    payload: op << 24,
                    a,
                    b,
                });
            } else if !stack.is_empty() && choice < 6 {
                nodes.push(Spec {
                    payload: OP_NEG << 24,
                    a: stack.pop(),
                    b: None,
                });
            } else {
                nodes.push(Spec {
                    payload: rng.next_u32() & CONST_MASK,
                    a: None,
                    b: None,
                });
            }
            stack.push(index);
        }
        while stack.len() > 1 {
            let index = nodes.len() as u32;
            let (b, a) = (stack.pop(), stack.pop());
            nodes.push(Spec {
                payload: OP_ADD << 24,
                a,
                b,
            });
            stack.push(index);
        }
        Plan { nodes }
    }
}

/// Evaluates a plan directly, independent of every variant. Returns the root's value.
pub fn reference_eval(plan: &Plan) -> u32 {
    let mut values: Vec<u32> = Vec::with_capacity(plan.nodes.len());
    for spec in &plan.nodes {
        let value_of = |child: Option<u32>| child.map_or(0, |i| values[i as usize]);
        let value = match spec.payload >> 24 {
            OP_NEG => value_of(spec.a).wrapping_neg(),
            OP_ADD => value_of(spec.a).wrapping_add(value_of(spec.b)),
            OP_MUL => value_of(spec.a).wrapping_mul(value_of(spec.b)),
            _ => spec.payload & CONST_MASK,
        };
        values.push(value);
    }
    values.last().copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_order(plan: &Plan) -> Vec<u32> {
        let mut out = Vec::new();
        let mut stack: Vec<u32> = Vec::new();
        let mut current = plan.root().map(|r| r as u32);
        while current.is_some() || !stack.is_empty() {
            while let Some(i) = current {
                stack.push(i);
                current = plan.nodes[i as usize].a;
            }
            let i = stack.pop().unwrap();
            out.push(plan.nodes[i as usize].payload);
            current = plan.nodes[i as usize].b;
        }
        out
    }

    fn assert_valid_tree(plan: &Plan) {
        let mut parents = vec![0u32; plan.nodes.len()];
        for (index, spec) in plan.nodes.iter().enumerate() {
            for child in [spec.a, spec.b].into_iter().flatten() {
                assert!(
                    (child as usize) < index,
                    "child {child} must precede parent {index}"
                );
                parents[child as usize] += 1;
            }
        }
        let root = plan.root().unwrap();
        for (index, count) in parents.iter().enumerate() {
            assert_eq!(
                *count,
                u32::from(index != root),
                "node {index} parent count"
            );
        }
    }

    #[test]
    fn balanced_bst_is_a_valid_tree_with_sorted_in_order_keys() {
        for n in (0..=40).chain([1000]) {
            let plan = Plan::balanced_bst(n);
            assert_eq!(plan.nodes.len(), n);
            assert_eq!(
                in_order(&plan),
                (0..n as u32).collect::<Vec<_>>(),
                "n = {n}"
            );
            if n > 0 {
                assert_valid_tree(&plan);
            }
        }
        assert_eq!(Plan::balanced_bst(0).root(), None);
    }

    #[test]
    fn balanced_bst_has_logarithmic_height_and_far_root_links() {
        let plan = Plan::balanced_bst(1024);
        let reach = plan.reach();
        assert_eq!(reach.links, 1023);
        assert!(
            reach.max_abs_delta >= 512,
            "the root's left child is far away"
        );
        assert!(
            reach.within_127 > 0.9,
            "most links join neighbours near the leaves"
        );
    }

    #[test]
    fn expression_is_a_valid_deterministic_tree() {
        for seed in [1, 2, 99] {
            for n in [1, 2, 3, 50, 1000] {
                let plan = Plan::expression(n, seed);
                assert!(plan.nodes.len() >= n);
                assert_valid_tree(&plan);
                assert_eq!(plan, Plan::expression(n, seed));
            }
        }
        assert_ne!(Plan::expression(100, 1), Plan::expression(100, 2));
    }

    #[test]
    fn expression_uses_every_operation() {
        let plan = Plan::expression(2000, 5);
        let ops: std::collections::BTreeSet<u32> =
            plan.nodes.iter().map(|s| s.payload >> 24).collect();
        assert_eq!(ops, [OP_LEAF, OP_ADD, OP_NEG, OP_MUL].into_iter().collect());
    }

    #[test]
    fn reference_eval_follows_the_documented_semantics() {
        let leaf = |c: u32| Spec {
            payload: c,
            a: None,
            b: None,
        };
        let plan = Plan {
            nodes: vec![
                leaf(7),
                leaf(5),
                Spec {
                    payload: OP_MUL << 24,
                    a: Some(0),
                    b: Some(1),
                }, // 35
                Spec {
                    payload: OP_NEG << 24,
                    a: Some(2),
                    b: None,
                }, // -35
                leaf(40),
                Spec {
                    payload: OP_ADD << 24,
                    a: Some(3),
                    b: Some(4),
                }, // 5
            ],
        };
        assert_eq!(reference_eval(&plan), 5);
        assert_eq!(reference_eval(&Plan { nodes: vec![] }), 0);
    }
}
