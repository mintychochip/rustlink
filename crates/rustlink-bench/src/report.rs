//! Markdown rendering of benchmark results.

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::fs;
use std::process::Command;

use crate::measure::Measurement;
use crate::proc::mem_available_kib;
use crate::registry;

/// Machine description recorded in the report header.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MachineInfo {
    /// CPU model name.
    pub cpu_model: String,
    /// The CPU the benchmark processes were pinned to.
    pub pinned_cpu: usize,
    /// Cache levels of the pinned CPU, e.g. `L1d 48K`.
    pub caches: Vec<String>,
    /// Kernel release.
    pub kernel: String,
    /// `rustc --version`.
    pub rustc: String,
    /// Short git revision of the checkout.
    pub git_rev: String,
    /// `MemAvailable` when the report was made.
    pub mem_available_kib: u64,
}

impl MachineInfo {
    /// Gathers the description of this machine. Best effort: missing facts stay empty.
    pub fn collect(pinned_cpu: usize) -> MachineInfo {
        let git_rev = command_output("git", &["rev-parse", "--short", "HEAD"]);
        MachineInfo {
            cpu_model: cpu_model(),
            pinned_cpu,
            caches: caches(pinned_cpu),
            kernel: command_output("uname", &["-r"]),
            rustc: command_output("rustc", &["--version"]),
            git_rev: if git_rev.is_empty() {
                "unknown".to_owned()
            } else {
                git_rev
            },
            mem_available_kib: mem_available_kib().unwrap_or(0),
        }
    }
}

fn command_output(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_default()
}

fn cpu_model() -> String {
    fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| {
                let rest = line.strip_prefix("model name")?;
                Some(rest.split_once(':')?.1.trim().to_owned())
            })
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

fn caches(cpu: usize) -> Vec<String> {
    let mut levels: Vec<(u32, String)> = Vec::new();
    for index in 0..8 {
        let dir = format!("/sys/devices/system/cpu/cpu{cpu}/cache/index{index}");
        let read = |name: &str| {
            fs::read_to_string(format!("{dir}/{name}"))
                .ok()
                .map(|s| s.trim().to_owned())
        };
        let (Some(level), Some(kind), Some(size)) = (read("level"), read("type"), read("size"))
        else {
            continue;
        };
        if kind == "Instruction" {
            continue;
        }
        let suffix = if kind == "Data" { "d" } else { "" };
        levels.push((
            level.parse().unwrap_or(0),
            format!("L{level}{suffix} {size}"),
        ));
    }
    levels.sort();
    levels.into_iter().map(|(_, text)| text).collect()
}

fn unique_in_order<'a>(items: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut seen = BTreeSet::new();
    items.filter(|item| seen.insert(*item)).collect()
}

fn size_label(n: u64) -> String {
    if n.is_power_of_two() {
        format!("2^{}", n.trailing_zeros())
    } else {
        n.to_string()
    }
}

/// Renders the whole report as Markdown. Pure: no I/O.
///
/// When a cell appears more than once the last measurement wins.
pub fn render(measurements: &[Measurement], info: &MachineInfo) -> String {
    let mut out = String::from("# rustlink benchmark results\n\n");
    let _ = writeln!(
        out,
        "- CPU: {} (processes pinned to cpu{})",
        info.cpu_model, info.pinned_cpu
    );
    if !info.caches.is_empty() {
        let _ = writeln!(
            out,
            "- Caches of the pinned CPU: {}",
            info.caches.join(", ")
        );
    }
    let _ = writeln!(out, "- Kernel: {}", info.kernel);
    let _ = writeln!(out, "- Compiler: {}", info.rustc);
    let _ = writeln!(out, "- Git revision: {}", info.git_rev);
    let _ = writeln!(
        out,
        "- MemAvailable: {:.1} GiB\n",
        info.mem_available_kib as f64 / 1_048_576.0
    );

    let mut latest = latest_per_cell(measurements);
    sort_in_registry_order(&mut latest);
    for workload in unique_in_order(latest.iter().map(|m| m.workload.as_str())) {
        let cells: Vec<&Measurement> = latest
            .iter()
            .copied()
            .filter(|m| m.workload == workload)
            .collect();
        render_workload(&mut out, workload, &cells);
    }
    // The gate compares `ast-bytes` with `ast`; a report of other workloads has nothing to say.
    if latest
        .iter()
        .any(|m| m.workload == "ast" || m.workload == "ast-bytes")
    {
        render_gate(&mut out, &latest);
    }
    out
}

/// The last measurement of every (workload, variant, n) cell, in the order the cells first appear.
fn latest_per_cell(measurements: &[Measurement]) -> Vec<&Measurement> {
    let mut slot: HashMap<(&str, &str, u64), usize> = HashMap::new();
    let mut latest: Vec<&Measurement> = Vec::new();
    for m in measurements {
        let key = (m.workload.as_str(), m.variant.as_str(), m.n);
        let index = *slot.entry(key).or_insert(latest.len());
        if index == latest.len() {
            latest.push(m);
        } else {
            latest[index] = m;
        }
    }
    latest
}

/// Orders cells as the registry lists them: workloads as registered, each workload's variants as
/// it declares them, sizes ascending. A sweep runs its cells in shuffled order, so the order of
/// the results says nothing about the order of the report. Names the registry does not know sort
/// last, alphabetically.
fn sort_in_registry_order(cells: &mut [&Measurement]) {
    type Registered<'r> = Vec<(&'r str, &'r [&'r str])>;
    fn rank<'a>(
        registered: &Registered,
        m: &'a Measurement,
    ) -> (usize, &'a str, usize, &'a str, u64) {
        let workload = registered.iter().position(|(name, _)| *name == m.workload);
        let variant = workload.and_then(|w| registered[w].1.iter().position(|v| *v == m.variant));
        (
            workload.unwrap_or(usize::MAX),
            m.workload.as_str(),
            variant.unwrap_or(usize::MAX),
            m.variant.as_str(),
            m.n,
        )
    }

    let registered: Registered = registry::all()
        .iter()
        .map(|workload| (workload.name(), workload.variants()))
        .collect();
    cells.sort_by(|a, b| rank(&registered, a).cmp(&rank(&registered, b)));
}

fn render_workload(out: &mut String, workload: &str, cells: &[&Measurement]) {
    let variants = unique_in_order(cells.iter().map(|m| m.variant.as_str()));
    let sizes: Vec<u64> = cells
        .iter()
        .map(|m| m.n)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let unit = cells
        .iter()
        .find(|m| m.status == "done")
        .map_or("unit", |m| m.unit.as_str());
    let _ = writeln!(out, "## {workload}\n");

    let mut table = |title: &str, value: &dyn Fn(&Measurement) -> String| {
        write_table(out, title, &variants, &sizes, cells, value);
    };
    table(&format!("Traverse (ns per {unit}, median ± MAD)"), &|m| {
        format!("{:.1} ± {:.1}", m.traverse_ns_median, m.traverse_ns_mad)
    });
    table("Build (ns per node, median ± MAD)", &|m| {
        format!("{:.1} ± {:.1}", m.build_ns_median, m.build_ns_mad)
    });
    table("Heap (bytes per node)", &|m| {
        format!("{:.1}", m.heap_bytes_per_node)
    });
    table("Peak RSS (MiB)", &|m| {
        format!("{:.1}", m.peak_rss_kib as f64 / 1024.0)
    });
    table("Node size (bytes)", &|m| m.node_bytes.to_string());

    let notes: Vec<String> = cells
        .iter()
        .filter(|m| m.status != "done")
        .map(|m| {
            let note = if m.note.is_empty() {
                m.status.as_str()
            } else {
                m.note.as_str()
            };
            format!("- `{}` at {}: {note}", m.variant, size_label(m.n))
        })
        .collect();
    if !notes.is_empty() {
        let _ = writeln!(out, "### Notes\n");
        for note in notes {
            let _ = writeln!(out, "{note}");
        }
        out.push('\n');
    }
}

fn write_table(
    out: &mut String,
    title: &str,
    variants: &[&str],
    sizes: &[u64],
    cells: &[&Measurement],
    value: &dyn Fn(&Measurement) -> String,
) {
    let _ = writeln!(out, "### {title}\n");
    let header: Vec<String> = sizes.iter().map(|&n| size_label(n)).collect();
    let _ = writeln!(out, "| variant | {} |", header.join(" | "));
    let _ = writeln!(out, "|---|{}|", vec!["---:"; sizes.len()].join("|"));
    for variant in variants {
        let row: Vec<String> = sizes
            .iter()
            .map(
                |&n| match cells.iter().find(|m| m.variant == *variant && m.n == n) {
                    None => "-".to_owned(),
                    Some(m) => match m.status.as_str() {
                        "done" => value(m),
                        "unreachable" => "n/a".to_owned(),
                        "skipped" => "skip".to_owned(),
                        _ => "fail".to_owned(),
                    },
                },
            )
            .collect();
        let _ = writeln!(out, "| {variant} | {} |", row.join(" | "));
    }
    out.push('\n');
}

fn render_gate(out: &mut String, measurements: &[&Measurement]) {
    let _ = writeln!(out, "## Layer-4 gate\n");
    let _ = writeln!(
        out,
        "Rule: proceed to layer 4 only if `ast-bytes` uses at least 1.5x less heap than the best \
         layer-2 variant (`abs*` or `rel*`: lowest heap bytes per node, the faster traversal among \
         equals) at no more than 1.1x its traversal time, at N >= 2^20.\n"
    );
    let sizes: BTreeSet<u64> = measurements
        .iter()
        .filter(|m| m.workload == "ast-bytes" && m.status == "done" && m.n >= 1 << 20)
        .map(|m| m.n)
        .collect();

    let mut rows = Vec::new();
    for n in sizes {
        let Some(bytes) = measurements
            .iter()
            .find(|m| m.workload == "ast-bytes" && m.n == n && m.status == "done")
        else {
            continue;
        };
        let candidates: Vec<&Measurement> = measurements
            .iter()
            .copied()
            .filter(|m| {
                m.workload == "ast"
                    && m.n == n
                    && m.status == "done"
                    && (m.variant.starts_with("abs") || m.variant.starts_with("rel"))
            })
            .collect();
        let Some(best) = candidates.iter().min_by(|a, b| {
            a.heap_bytes_per_node
                .total_cmp(&b.heap_bytes_per_node)
                .then(a.traverse_ns_median.total_cmp(&b.traverse_ns_median))
        }) else {
            continue;
        };
        let memory_ratio = if bytes.heap_bytes_per_node > 0.0 {
            best.heap_bytes_per_node / bytes.heap_bytes_per_node
        } else {
            0.0
        };
        let time_ratio = bytes.traverse_ns_median / best.traverse_ns_median;
        let verdict = if memory_ratio >= 1.5 && time_ratio <= 1.1 {
            "pass"
        } else {
            "fail"
        };
        rows.push(format!(
            "| {} | {} | {:.1} | {:.1} | {:.2} | {:.1} | {:.1} | {:.2} | {verdict} |",
            size_label(n),
            best.variant,
            best.heap_bytes_per_node,
            bytes.heap_bytes_per_node,
            memory_ratio,
            best.traverse_ns_median,
            bytes.traverse_ns_median,
            time_ratio,
        ));
    }

    if rows.is_empty() {
        let _ = writeln!(
            out,
            "No data for the gate (needs ast and ast-bytes at N >= 2^20)."
        );
        return;
    }
    let _ = writeln!(
        out,
        "| N | best L2 variant | its heap B/node | ast-bytes heap B/node | memory ratio | its traverse ns | \
         ast-bytes traverse ns | time ratio | verdict |"
    );
    let _ = writeln!(out, "|---|---|---:|---:|---:|---:|---:|---:|---|");
    for row in rows {
        let _ = writeln!(out, "{row}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(workload: &str, variant: &str, n: u64, status: &str) -> Measurement {
        Measurement {
            workload: workload.to_owned(),
            variant: variant.to_owned(),
            n,
            status: status.to_owned(),
            unit: "lookup".to_owned(),
            node_bytes: 12,
            heap_bytes_per_node: 12.0,
            build_ns_median: 50.0,
            build_ns_mad: 1.0,
            traverse_ns_median: 100.0,
            traverse_ns_mad: 2.5,
            peak_rss_kib: 2048,
            ..Measurement::default()
        }
    }

    fn info() -> MachineInfo {
        MachineInfo {
            cpu_model: "Test CPU".to_owned(),
            pinned_cpu: 3,
            caches: vec!["L1d 48K".to_owned(), "L3 36864K".to_owned()],
            kernel: "7.1.0".to_owned(),
            rustc: "rustc 1.99.0-nightly".to_owned(),
            git_rev: "abc1234".to_owned(),
            mem_available_kib: 2_097_152,
        }
    }

    #[test]
    fn header_lists_the_machine() {
        let text = render(&[], &info());
        assert!(text.starts_with("# rustlink benchmark results\n"));
        assert!(text.contains("- CPU: Test CPU (processes pinned to cpu3)"));
        assert!(text.contains("- Caches of the pinned CPU: L1d 48K, L3 36864K"));
        assert!(text.contains("- Git revision: abc1234"));
        assert!(text.contains("- MemAvailable: 2.0 GiB"));
    }

    #[test]
    fn tables_show_values_and_mark_unreachable_skipped_failed_and_missing_cells() {
        let mut unreachable = cell("bst-bulk", "rel8", 1 << 12, "unreachable");
        unreachable.note = "links do not fit".to_owned();
        let mut skipped = cell("bst-bulk", "box", 1 << 12, "skipped");
        skipped.note = "too slow".to_owned();
        let failed = cell("bst-bulk", "usize", 1 << 12, "failed");
        let ms = vec![
            cell("bst-bulk", "u32", 1 << 10, "done"),
            cell("bst-bulk", "u32", 1 << 12, "done"),
            cell("bst-bulk", "rel8", 1 << 10, "done"),
            unreachable,
            skipped,
            failed,
        ];
        let text = render(&ms, &info());
        assert!(text.contains("## bst-bulk"));
        assert!(text.contains("### Traverse (ns per lookup, median ± MAD)"));
        assert!(text.contains("| variant | 2^10 | 2^12 |"));
        assert!(text.contains("| u32 | 100.0 ± 2.5 | 100.0 ± 2.5 |"));
        assert!(text.contains("| rel8 | 100.0 ± 2.5 | n/a |"));
        assert!(text.contains("| box | - | skip |"));
        assert!(text.contains("| usize | - | fail |"));
        assert!(text.contains("### Heap (bytes per node)"));
        assert!(text.contains("| u32 | 12.0 | 12.0 |"));
        assert!(text.contains("### Peak RSS (MiB)"));
        assert!(text.contains("| u32 | 2.0 | 2.0 |"));
        assert!(text.contains("### Node size (bytes)"));
    }

    #[test]
    fn notes_explain_every_cell_that_did_not_finish() {
        let mut unreachable = cell("ast", "rel8", 1 << 20, "unreachable");
        unreachable.note = "max distance 9000".to_owned();
        let failed = cell("ast", "box", 1 << 20, "failed");
        let text = render(
            &[unreachable, failed, cell("ast", "u32", 1 << 20, "done")],
            &info(),
        );
        assert!(text.contains("### Notes"));
        assert!(text.contains("- `rel8` at 2^20: max distance 9000"));
        assert!(text.contains("- `box` at 2^20: failed"));
    }

    #[test]
    fn no_notes_section_when_everything_finished() {
        assert!(!render(&[cell("ast", "u32", 1024, "done")], &info()).contains("### Notes"));
    }

    #[test]
    fn non_power_of_two_sizes_print_as_numbers_and_the_last_measurement_wins() {
        let mut first = cell("w", "v", 1000, "done");
        first.traverse_ns_median = 1.0;
        let mut second = first.clone();
        second.traverse_ns_median = 2.0;
        let text = render(&[first, second], &info());
        assert!(text.contains("| variant | 1000 |"));
        assert!(text.contains("| v | 2.0 ± 2.5 |"));
    }

    fn gate_inputs(l2_heap: f64, l2_ns: f64, bytes_heap: f64, bytes_ns: f64) -> Vec<Measurement> {
        let mut weak = cell("ast", "abs32", 1 << 20, "done");
        weak.heap_bytes_per_node = 12.0;
        let mut best = cell("ast", "rel16", 1 << 20, "done");
        best.heap_bytes_per_node = l2_heap;
        best.traverse_ns_median = l2_ns;
        let mut baseline = cell("ast", "u32", 1 << 20, "done");
        baseline.heap_bytes_per_node = 1.0; // not a layer-2 variant: ignored
        let mut bytes = cell("ast-bytes", "bytes", 1 << 20, "done");
        bytes.heap_bytes_per_node = bytes_heap;
        bytes.traverse_ns_median = bytes_ns;
        vec![weak, best, baseline, bytes]
    }

    #[test]
    fn gate_passes_when_memory_drops_enough_without_slowing_down() {
        let text = render(&gate_inputs(8.0, 100.0, 4.0, 105.0), &info());
        assert!(text.contains("## Layer-4 gate"));
        assert!(text.contains("| 2^20 | rel16 | 8.0 | 4.0 | 2.00 | 100.0 | 105.0 | 1.05 | pass |"));
    }

    #[test]
    fn gate_fails_when_memory_gain_is_too_small_or_traversal_too_slow() {
        let small_gain = render(&gate_inputs(8.0, 100.0, 6.0, 100.0), &info());
        assert!(small_gain.contains("| 1.33 |"));
        assert!(small_gain.contains("| fail |"));
        let slow = render(&gate_inputs(8.0, 100.0, 4.0, 130.0), &info());
        assert!(slow.contains("| 1.30 | fail |"));
    }

    #[test]
    fn gate_reports_missing_data() {
        let text = render(&[cell("ast", "rel16", 1 << 12, "done")], &info());
        assert!(text.contains("No data for the gate (needs ast and ast-bytes at N >= 2^20)."));
        let below_threshold = {
            let mut ms = gate_inputs(8.0, 100.0, 4.0, 100.0);
            ms.iter_mut().for_each(|m| m.n = 1 << 18);
            render(&ms, &info())
        };
        assert!(below_threshold.contains("No data for the gate"));
    }

    #[test]
    fn a_cell_that_ran_again_is_judged_by_its_last_measurement_in_the_gate() {
        let mut ms = gate_inputs(8.0, 100.0, 4.0, 105.0);
        let mut again = ms[1].clone(); // the best layer-2 variant, measured a second time
        again.traverse_ns_median = 50.0;
        ms.push(again);
        let text = render(&ms, &info());
        assert!(text.contains("| 2^20 | rel16 | 8.0 | 4.0 | 2.00 | 50.0 | 105.0 | 2.10 | fail |"));
    }

    #[test]
    fn a_later_run_replaces_the_failure_it_repeats() {
        let ms = [
            cell("ast", "box", 1 << 20, "failed"),
            cell("ast", "box", 1 << 20, "done"),
        ];
        assert!(!render(&ms, &info()).contains("### Notes"));
    }

    #[test]
    fn equally_small_layer_two_variants_are_ranked_by_traversal_time_in_either_order() {
        let timed = |variant: &str, ns: f64| {
            let mut m = cell("ast", variant, 1 << 20, "done");
            m.traverse_ns_median = ns;
            m
        };
        let mut bytes = cell("ast-bytes", "bytes", 1 << 20, "done");
        bytes.heap_bytes_per_node = 6.0;
        bytes.traverse_ns_median = 100.0;
        for ms in [
            [timed("abs32", 100.0), timed("rel32", 90.0), bytes.clone()],
            [timed("rel32", 90.0), timed("abs32", 100.0), bytes.clone()],
        ] {
            let text = render(&ms, &info());
            assert!(
                text.contains("| 2^20 | rel32 | 12.0 | 6.0 | 2.00 | 90.0 | 100.0 | 1.11 | fail |")
            );
        }
    }

    #[test]
    fn workloads_variants_sizes_and_notes_follow_registry_order_whatever_order_cells_ran_in() {
        // A sweep runs its cells in shuffled order and appends them in that order.
        let mut unreachable_small = cell("bst-bulk", "rel8", 1 << 10, "unreachable");
        unreachable_small.note = "small".to_owned();
        let mut unreachable_large = cell("bst-bulk", "rel8", 1 << 12, "unreachable");
        unreachable_large.note = "large".to_owned();
        let ms = [
            cell("ast", "box", 1 << 10, "done"),
            unreachable_large,
            cell("bst-bulk", "u32", 1 << 12, "done"),
            cell("ast", "abs32", 1 << 10, "done"),
            cell("bst-bulk", "box", 1 << 10, "done"),
            unreachable_small,
            cell("bst-bulk", "u32", 1 << 10, "done"),
        ];
        let text = render(&ms, &info());
        let at = |needle: &str| {
            text.find(needle)
                .unwrap_or_else(|| panic!("{needle} missing"))
        };

        assert!(
            at("## bst-bulk") < at("## ast"),
            "workloads follow the registry"
        );
        let table = &text[at("## bst-bulk")..at("## ast")];
        let row = |variant: &str| table.find(&format!("| {variant} |")).expect("row");
        assert!(
            row("box") < row("u32") && row("u32") < row("rel8"),
            "variants follow the workload"
        );
        assert!(table.contains("| variant | 2^10 | 2^12 |"), "sizes ascend");
        assert!(
            at("`rel8` at 2^10") < at("`rel8` at 2^12"),
            "notes ascend by size"
        );
    }

    #[test]
    fn a_report_without_expression_tree_data_has_no_layer_4_gate() {
        let text = render(&[cell("ngraph-mesh", "u32", 1 << 12, "done")], &info());
        assert!(text.contains("## ngraph-mesh"));
        assert!(!text.contains("Layer-4 gate"));
    }

    #[test]
    fn collect_never_fails_and_pins_the_requested_cpu() {
        let info = MachineInfo::collect(0);
        assert_eq!(info.pinned_cpu, 0);
        assert!(!info.git_rev.is_empty());
    }
}
