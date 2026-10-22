//! Tree workloads: `bst-random`, `bst-bulk`, `ast` and `ast-bytes`.
//! Owned by the bench-trees task.

use crate::workload::Workload;

/// The workloads this module provides.
pub fn workloads() -> Vec<Box<dyn Workload>> {
    Vec::new()
}
