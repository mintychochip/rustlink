//! The list of registered workloads. Each workload module exposes a `workloads()` function.

use crate::workload::Workload;
use crate::{graph, trees};

/// Every registered workload, in report order.
pub fn all() -> Vec<Box<dyn Workload>> {
    let mut all = trees::workloads();
    all.extend(graph::workloads());
    all
}

/// The workload called `name`, if registered.
pub fn find(name: &str) -> Option<Box<dyn Workload>> {
    all().into_iter().find(|workload| workload.name() == name)
}
