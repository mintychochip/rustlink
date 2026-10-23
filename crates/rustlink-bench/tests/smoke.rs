//! End-to-end smoke tests of the `rlb` and `sweep` binaries against the real registry.

use std::path::PathBuf;
use std::process::Command;

use rustlink_bench::graph::Graph;
use rustlink_bench::measure::Measurement;
use rustlink_bench::registry;

fn rlb(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rlb"))
        .args(args)
        .output()
        .expect("rlb starts")
}

#[test]
fn every_workload_and_variant_runs_at_n_1024_and_prints_one_valid_json_line() {
    let mut cells = 0;
    for workload in registry::all() {
        for &variant in workload.variants() {
            let output = rlb(&[
                "run",
                "--workload",
                workload.name(),
                "--variant",
                variant,
                "--n",
                "1024",
                "--reps",
                "3",
            ]);
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success(),
                "{} {variant}: {}",
                workload.name(),
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                stdout.lines().count(),
                1,
                "{} {variant}: {stdout}",
                workload.name()
            );
            let m = Measurement::from_json(stdout.trim()).unwrap();
            assert_eq!(
                (m.workload.as_str(), m.variant.as_str(), m.n),
                (workload.name(), variant, 1024)
            );
            assert!(
                matches!(m.status.as_str(), "done" | "unreachable"),
                "{} {variant}: {} {}",
                workload.name(),
                m.status,
                m.note
            );
            assert!(m.peak_rss_kib > 0);
            if m.status == "done" {
                assert!(
                    m.traverse_ns_median > 0.0 && m.node_bytes > 0,
                    "{} {variant}",
                    workload.name()
                );
                assert!(
                    m.heap_bytes_per_node > 0.0,
                    "{} {variant}: counting allocator",
                    workload.name()
                );
            }
            cells += 1;
        }
    }
    assert!(
        cells >= 9,
        "every registered workload has at least one variant"
    );
}

#[test]
fn list_describes_every_registered_workload() {
    let output = rlb(&["list"]);
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert_eq!(text.lines().count(), registry::all().len());
    assert!(text.lines().any(|l| l.starts_with("bst-random\t4194304\t")));
}

#[test]
fn invalid_requests_exit_with_status_2_and_print_nothing_on_stdout() {
    for args in [
        &["run", "--workload", "nope", "--variant", "u32", "--n", "8"][..],
        &["run", "--workload", "ast", "--variant", "nope", "--n", "8"][..],
        &["run", "--workload", "ast"][..],
        &["frobnicate"][..],
    ] {
        let output = rlb(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(!output.stderr.is_empty(), "{args:?}");
    }
}

#[test]
fn a_size_above_max_n_is_reported_as_skipped() {
    let output = rlb(&[
        "run",
        "--workload",
        "graph-csr",
        "--variant",
        "u32",
        "--n",
        "100000000",
    ]);
    assert!(output.status.success());
    let m = Measurement::from_json(String::from_utf8_lossy(&output.stdout).trim()).unwrap();
    assert_eq!(m.status, "skipped");
    assert!(m.note.contains("max_n"));
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rustlink-bench-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn sweep_runs_a_tiny_matrix_and_renders_the_report() {
    let dir = temp_dir("sweep");
    let (jsonl, md) = (dir.join("r.jsonl"), dir.join("r.md"));
    let sweep = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_sweep"))
            .args([
                "--out",
                jsonl.to_str().unwrap(),
                "--md",
                md.to_str().unwrap(),
            ])
            .args(extra)
            .output()
            .expect("sweep starts")
    };

    let output = sweep(&[
        "--sizes",
        "10",
        "--reps",
        "3",
        "--workloads",
        "ast,ast-bytes",
        "--variants",
        "u32,rel8,bytes",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let lines = std::fs::read_to_string(&jsonl).unwrap();
    let measurements: Vec<Measurement> = lines
        .lines()
        .map(|l| Measurement::from_json(l).unwrap())
        .collect();
    // ast: u32 and rel8; ast-bytes: bytes.
    assert_eq!(measurements.len(), 3, "{lines}");
    let report = std::fs::read_to_string(&md).unwrap();
    assert!(report.contains("# rustlink benchmark results"));
    assert!(report.contains("## ast"));
    assert!(report.contains("## ast-bytes"));
    assert!(report.contains("## Layer-4 gate"));

    // --report-only re-renders from the existing results without running anything.
    std::fs::remove_file(&md).unwrap();
    assert!(sweep(&["--report-only"]).status.success());
    assert!(std::fs::read_to_string(&md).unwrap().contains("## ast"));
    assert_eq!(
        std::fs::read_to_string(&jsonl).unwrap(),
        lines,
        "report-only must not append"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sweep_dry_run_lists_cells_without_running_or_writing() {
    let dir = temp_dir("dry");
    let jsonl = dir.join("r.jsonl");
    let output = Command::new(env!("CARGO_BIN_EXE_sweep"))
        .args([
            "--dry-run",
            "--sizes",
            "10,12",
            "--workloads",
            "ast-bytes",
            "--out",
            jsonl.to_str().unwrap(),
        ])
        .output()
        .expect("sweep starts");
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.starts_with("cpu: "));
    assert_eq!(
        text.lines()
            .filter(|l| l.starts_with("run  ast-bytes bytes"))
            .count(),
        2
    );
    assert!(!jsonl.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Runs one cell through `rlb` and returns its measurement.
fn cell(workload: &str, variant: &str, n: usize) -> Measurement {
    let n = n.to_string();
    let output = rlb(&[
        "run",
        "--workload",
        workload,
        "--variant",
        variant,
        "--n",
        &n,
        "--reps",
        "2",
    ]);
    assert!(output.status.success(), "{workload} {variant}");
    Measurement::from_json(String::from_utf8_lossy(&output.stdout).trim()).unwrap()
}

/// Heap bytes per node must equal the exact size of the structure. Slack, such as a `Vec`
/// grown by doubling because `collect` could not know its length, would silently inflate one
/// variant's reported memory and bias every memory comparison against it.
#[test]
fn heap_bytes_per_node_equal_the_exact_structure_size() {
    let check = |m: &Measurement, expected: f64| {
        assert_eq!(m.status, "done", "{} {}", m.workload, m.variant);
        let error = (m.heap_bytes_per_node - expected).abs() / expected;
        assert!(
            error < 0.005,
            "{} {}: heap {:.3} B/node, structure is {:.3}",
            m.workload,
            m.variant,
            m.heap_bytes_per_node,
            expected
        );
    };
    let n = 4096;
    let graph = Graph::generate(n, 1); // rlb's default seed
    let (vertices, edges) = (n as f64, graph.edges() as f64);

    // Trees: one node per element and nothing else.
    for (variant, node_bytes) in [
        ("box", 24.0),
        ("usize", 24.0),
        ("u32", 12.0),
        ("abs32", 12.0),
        ("abs16", 8.0),
        ("rel32", 12.0),
        ("rel16", 8.0),
    ] {
        check(&cell("bst-bulk", variant, n), node_bytes);
    }

    // CSR: V + 1 offsets and one entry per edge, both as wide as the variant's index.
    for (variant, word) in [("usize", 8.0), ("u32", 4.0), ("abs32", 4.0)] {
        check(
            &cell("graph-csr", variant, n),
            ((vertices + 1.0) * word + edges * word) / vertices,
        );
    }

    // Linked lists: one head per vertex and one node per edge.
    for (variant, head, edge) in [
        ("box", 8.0, 16.0),
        ("usize", 8.0, 16.0),
        ("u32", 4.0, 8.0),
        ("abs32", 4.0, 8.0),
        ("rel32", 4.0, 8.0),
        ("abs16", 4.0, 6.0),
        ("rel16", 4.0, 6.0),
        ("rel8", 4.0, 5.0),
    ] {
        check(
            &cell("graph-linked-seq", variant, n),
            (vertices * head + edges * edge) / vertices,
        );
    }

    // Node graphs: one node per vertex; `Rc` adds its two counters (16 bytes, in the allocation
    // with a 40-byte node) and an 8-byte owner slot.
    for (variant, node_bytes) in [
        ("rc", 8.0 + 16.0 + 40.0),
        ("usize", 40.0),
        ("u32", 20.0),
        ("abs32", 20.0),
        ("rel32", 20.0),
        ("abs16", 12.0),
        ("rel16", 12.0),
        ("rel8", 8.0),
    ] {
        check(&cell("ngraph-mesh", variant, n), node_bytes);
    }
}
