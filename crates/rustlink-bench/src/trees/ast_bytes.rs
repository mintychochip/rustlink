//! `ast-bytes`: a bench-only prototype of a Goose-style byte layout, used for the layer-4 gate.
//!
//! The same expression tree as the `ast` workload is stored as variable-size records appended to
//! one `Vec<u8>` in creation order (children first). Links are LEB128 varints holding the
//! backward distance in bytes to the child's record, so small distances cost one byte. It is
//! not part of the library.

use crate::measure::{Params, Pass, run_reps};
use crate::plan::{CONST_MASK, OP_MUL, Plan};
use crate::workload::{Outcome, Workload};

const TAG_LEAF: u8 = 0;
const TAG_ADD: u8 = 1;
const TAG_NEG: u8 = 2;
const TAG_MUL: u8 = 3;

fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn get_varint(bytes: &[u8], at: &mut usize) -> u64 {
    let mut value = 0u64;
    let mut shift = 0;
    loop {
        let byte = bytes[*at];
        *at += 1;
        value |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return value;
        }
        shift += 7;
    }
}

/// An expression tree stored as variable-size byte records.
pub struct ByteTree {
    bytes: Vec<u8>,
    root: usize,
    nodes: usize,
}

impl ByteTree {
    /// Encodes `plan`.
    pub fn from_plan(plan: &Plan) -> ByteTree {
        let mut bytes = Vec::new();
        let mut starts: Vec<usize> = Vec::with_capacity(plan.nodes.len());
        for spec in &plan.nodes {
            let here = bytes.len();
            starts.push(here);
            match (spec.a, spec.b) {
                (None, _) => {
                    bytes.push(TAG_LEAF);
                    bytes.extend_from_slice(&(spec.payload & CONST_MASK).to_le_bytes()[..3]);
                }
                (Some(a), None) => {
                    bytes.push(TAG_NEG);
                    put_varint(&mut bytes, (here - starts[a as usize]) as u64);
                }
                (Some(a), Some(b)) => {
                    bytes.push(if spec.payload >> 24 == OP_MUL {
                        TAG_MUL
                    } else {
                        TAG_ADD
                    });
                    put_varint(&mut bytes, (here - starts[a as usize]) as u64);
                    put_varint(&mut bytes, (here - starts[b as usize]) as u64);
                }
            }
        }
        bytes.shrink_to_fit();
        ByteTree {
            root: starts.last().copied().unwrap_or(0),
            nodes: plan.nodes.len(),
            bytes,
        }
    }

    /// Total size of the records in bytes.
    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }

    /// Evaluates the tree with explicit stacks. Returns the value and the number of records
    /// visited, with the same semantics as the `ast` workload.
    pub fn eval(&self) -> (u32, u64) {
        enum Step {
            Enter(usize),
            Combine(u8),
        }
        if self.nodes == 0 {
            return (0, 0);
        }
        let mut stack = vec![Step::Enter(self.root)];
        let mut values: Vec<u32> = Vec::new();
        let mut visits = 0u64;
        while let Some(step) = stack.pop() {
            match step {
                Step::Enter(offset) => {
                    visits += 1;
                    let tag = self.bytes[offset];
                    let mut at = offset + 1;
                    match tag {
                        TAG_LEAF => values.push(u32::from_le_bytes([
                            self.bytes[at],
                            self.bytes[at + 1],
                            self.bytes[at + 2],
                            0,
                        ])),
                        TAG_NEG => {
                            let a = offset - get_varint(&self.bytes, &mut at) as usize;
                            stack.push(Step::Combine(tag));
                            stack.push(Step::Enter(a));
                        }
                        _ => {
                            let a = offset - get_varint(&self.bytes, &mut at) as usize;
                            let b = offset - get_varint(&self.bytes, &mut at) as usize;
                            stack.push(Step::Combine(tag));
                            stack.push(Step::Enter(b));
                            stack.push(Step::Enter(a));
                        }
                    }
                }
                Step::Combine(TAG_NEG) => {
                    let x = values.pop().expect("operand");
                    values.push(x.wrapping_neg());
                }
                Step::Combine(tag) => {
                    let y = values.pop().expect("second operand");
                    let x = values.pop().expect("first operand");
                    values.push(if tag == TAG_MUL {
                        x.wrapping_mul(y)
                    } else {
                        x.wrapping_add(y)
                    });
                }
            }
        }
        (values.pop().unwrap_or(0), visits)
    }
}

/// The byte-record prototype as a workload with the single variant `bytes`.
pub struct AstBytes;

impl Workload for AstBytes {
    fn name(&self) -> &'static str {
        "ast-bytes"
    }

    fn variants(&self) -> &'static [&'static str] {
        &["bytes"]
    }

    fn run(&self, variant: &str, params: &Params) -> Result<Outcome, String> {
        if variant != "bytes" {
            return Err(format!("unknown variant `{variant}`"));
        }
        let plan = Plan::expression(params.n.max(1), params.seed);
        let reach = plan.reach();
        let nodes = plan.nodes.len();
        let node_bytes =
            (ByteTree::from_plan(&plan).byte_len() as f64 / nodes as f64).round() as usize;
        Ok(Outcome::from_reps(
            run_reps(
                params,
                nodes as u64,
                || ByteTree::from_plan(&plan),
                |tree| {
                    let (value, visits) = tree.eval();
                    Pass {
                        checksum: u64::from(value),
                        units: visits,
                    }
                },
            ),
            node_bytes,
            "visit",
            Some(reach),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{OP_NEG, Spec, reference_eval};

    #[test]
    fn varints_round_trip_across_byte_boundaries() {
        for value in [
            0u64,
            1,
            127,
            128,
            300,
            16_383,
            16_384,
            u32::MAX as u64,
            u64::MAX >> 1,
        ] {
            let mut buffer = Vec::new();
            put_varint(&mut buffer, value);
            let mut at = 0;
            assert_eq!(get_varint(&buffer, &mut at), value);
            assert_eq!(at, buffer.len());
        }
        let mut one = Vec::new();
        put_varint(&mut one, 127);
        assert_eq!(one.len(), 1);
        put_varint(&mut one, 128);
        assert_eq!(one.len(), 3);
    }

    #[test]
    fn evaluation_matches_the_reference_evaluator() {
        for n in [1, 2, 3, 50, 1000, 4096] {
            let plan = Plan::expression(n, 7);
            let tree = ByteTree::from_plan(&plan);
            assert_eq!(
                tree.eval(),
                (reference_eval(&plan), plan.nodes.len() as u64),
                "n = {n}"
            );
        }
    }

    #[test]
    fn records_are_smaller_than_any_fixed_width_node() {
        let plan = Plan::expression(4096, 7);
        let tree = ByteTree::from_plan(&plan);
        let average = tree.byte_len() as f64 / plan.nodes.len() as f64;
        assert!(average < 6.0, "average record is {average} bytes");
    }

    #[test]
    fn a_deep_chain_is_evaluated_without_recursion() {
        let mut nodes = vec![Spec {
            payload: 9,
            a: None,
            b: None,
        }];
        nodes.extend((1..100_000u32).map(|i| Spec {
            payload: OP_NEG << 24,
            a: Some(i - 1),
            b: None,
        }));
        let plan = Plan { nodes };
        assert_eq!(
            ByteTree::from_plan(&plan).eval(),
            (reference_eval(&plan), 100_000)
        );
    }

    #[test]
    fn only_the_bytes_variant_exists() {
        let params = Params {
            n: 64,
            reps: 3,
            seed: 1,
            budget: std::time::Duration::from_secs(60),
        };
        assert!(AstBytes.run("u32", &params).is_err());
        let Outcome::Done(metrics) = AstBytes.run("bytes", &params).unwrap() else {
            panic!("expected Done")
        };
        assert!(metrics.node_bytes >= 2 && metrics.node_bytes <= 6);
        assert_eq!(metrics.unit, "visit");
    }

    #[test]
    fn an_empty_tree_evaluates_to_nothing() {
        assert_eq!(ByteTree::from_plan(&Plan { nodes: vec![] }).eval(), (0, 0));
    }
}
