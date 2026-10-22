//! Graph workloads: `graph-csr`, `graph-linked-seq` and `graph-linked-shuf`.
//! Owned by the bench-graph task.

use crate::workload::Workload;

/// The workloads this module provides.
pub fn workloads() -> Vec<Box<dyn Workload>> {
    Vec::new()
}
