//! The pointer baseline: every node is a separate heap allocation.

use super::{TreeBuild, TreeView};
use crate::plan::Plan;
use crate::workload::Limit;

/// One heap-allocated node.
pub struct Node {
    payload: u32,
    a: Option<Box<Node>>,
    b: Option<Box<Node>>,
}

/// A tree of boxed nodes.
pub struct BoxTree {
    root: Option<Box<Node>>,
    len: usize,
}

impl Drop for BoxTree {
    /// Drops iteratively so a very deep tree cannot overflow the call stack.
    fn drop(&mut self) {
        let mut stack: Vec<Box<Node>> = Vec::new();
        stack.extend(self.root.take());
        while let Some(mut node) = stack.pop() {
            stack.extend(node.a.take());
            stack.extend(node.b.take());
        }
    }
}

impl TreeView for BoxTree {
    type Ref<'a> = &'a Node;

    fn root(&self) -> Option<&Node> {
        self.root.as_deref()
    }

    fn payload(&self, r: &Node) -> u32 {
        r.payload
    }

    fn a<'a>(&'a self, r: &'a Node) -> Option<&'a Node> {
        r.a.as_deref()
    }

    fn b<'a>(&'a self, r: &'a Node) -> Option<&'a Node> {
        r.b.as_deref()
    }
}

impl TreeBuild for BoxTree {
    const NODE_BYTES: usize = size_of::<Node>();
    const LIMIT: Limit = Limit::None;

    fn empty(_capacity: usize) -> Self {
        BoxTree { root: None, len: 0 }
    }

    fn from_plan(plan: &Plan) -> Self {
        let mut slots: Vec<Option<Box<Node>>> = Vec::with_capacity(plan.nodes.len());
        for spec in &plan.nodes {
            let a = spec.a.and_then(|i| slots[i as usize].take());
            let b = spec.b.and_then(|i| slots[i as usize].take());
            slots.push(Some(Box::new(Node {
                payload: spec.payload,
                a,
                b,
            })));
        }
        BoxTree {
            root: slots.pop().flatten(),
            len: plan.nodes.len(),
        }
    }

    fn insert_bst(&mut self, key: u32) {
        let mut slot = &mut self.root;
        while let Some(node) = slot {
            slot = match key.cmp(&node.payload) {
                core::cmp::Ordering::Less => &mut node.a,
                core::cmp::Ordering::Greater => &mut node.b,
                core::cmp::Ordering::Equal => return,
            };
        }
        *slot = Some(Box::new(Node {
            payload: key,
            a: None,
            b: None,
        }));
        self.len += 1;
    }

    fn len(&self) -> usize {
        self.len
    }
}
