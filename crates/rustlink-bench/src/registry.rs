//! The list of registered workloads. Each workload module exposes a `workloads()` function.

use crate::workload::Workload;
use crate::{blob_load, graph, trees};

/// Every registered workload, in report order.
pub fn all() -> Vec<Box<dyn Workload>> {
    let mut all = trees::workloads();
    all.extend(graph::workloads());
    all.extend(blob_load::workloads());
    all
}

/// The workload called `name`, if registered.
pub fn find(name: &str) -> Option<Box<dyn Workload>> {
    all().into_iter().find(|workload| workload.name() == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_every_workload_once_in_report_order() {
        let names: Vec<&str> = all().iter().map(|w| w.name()).collect();
        assert_eq!(
            names,
            [
                "bst-random",
                "bst-bulk",
                "ast",
                "ast-bytes",
                "graph-csr",
                "graph-linked-seq",
                "graph-linked-shuf",
                "blob-load",
                "blob-lookup",
            ]
        );
    }

    #[test]
    fn find_looks_workloads_up_by_name() {
        assert_eq!(find("ast").map(|w| w.name()), Some("ast"));
        assert!(find("nope").is_none());
    }
}
