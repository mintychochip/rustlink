//! Benchmarks comparing pointer, index and narrow-link data structures.
//!
//! A [`workload::Workload`] runs one data-structure task for several link representations
//! ("variants"). [`measure::run_reps`] times build and traversal; the `rlb` binary runs one cell
//! in a fresh process and prints a [`measure::Measurement`]; the `sweep` binary runs the whole
//! matrix and renders `bench/results.md`.

#![deny(unsafe_code)]

pub mod alloc;
pub mod blob_load;
pub mod btree;
pub mod containers;
pub mod graph;
pub mod measure;
pub mod ngraph;
pub mod plan;
pub mod proc;
pub mod registry;
pub mod report;
pub mod rng;
pub mod sets;
pub mod stats;
pub mod trees;
pub mod variants;
pub mod workload;
