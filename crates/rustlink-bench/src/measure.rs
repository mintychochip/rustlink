//! The run loop and the measurement record exchanged between `rlb`, `sweep` and the report.

use std::hint::black_box;
use std::time::{Duration, Instant};

use crate::alloc;
use crate::stats::{Dist, dist};
use crate::workload::{Outcome, Reach};

/// Parameters shared by every workload run.
#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    /// Workload size: nodes for trees, vertices for graphs.
    pub n: usize,
    /// Repetitions including one discarded warm-up; at least 2.
    pub reps: usize,
    /// Seed for every random input.
    pub seed: u64,
    /// The cell is skipped if the warm-up repetition predicts the remaining ones exceed this.
    pub budget: Duration,
}

impl Params {
    /// Default parameters for size `n`: 6 repetitions (5 measured), seed 1, 120 s budget.
    pub fn new(n: usize) -> Self {
        Params {
            n,
            reps: 6,
            seed: 1,
            budget: Duration::from_secs(120),
        }
    }
}

/// What one traversal pass did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pass {
    /// A value derived from the work; consumed with `black_box` so it cannot be optimised away.
    pub checksum: u64,
    /// Units of work done (lookups, node visits or edge scans); the traverse metric is ns per unit.
    pub units: u64,
}

/// Statistics over the measured repetitions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RepStats {
    /// Live heap bytes requested per node after the build (0 without the counting allocator).
    pub heap_bytes_per_node: f64,
    /// Build time in ns per node.
    pub build: Dist,
    /// Traversal time in ns per unit of work.
    pub traverse: Dist,
}

/// The warm-up repetition predicted the cell would exceed its time budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverBudget {
    /// Predicted time of the remaining repetitions.
    pub predicted: Duration,
    /// The budget.
    pub budget: Duration,
}

/// Runs `build` and `traverse` `p.reps` times. Repetition 0 is a warm-up: it runs with the
/// counting allocator enabled to measure heap bytes, and its duration decides whether the cell
/// fits the time budget. The other repetitions are timed with counting off.
///
/// `nodes` normalises build time; each pass reports its own unit count for traversal time.
pub fn run_reps<S>(
    p: &Params,
    nodes: u64,
    mut build: impl FnMut() -> S,
    mut traverse: impl FnMut(&S) -> Pass,
) -> Result<RepStats, OverBudget> {
    assert!(
        p.reps >= 2,
        "need at least one warm-up and one measured repetition"
    );
    let nodes = nodes.max(1);

    let started = Instant::now();
    alloc::set_counting(true);
    let before = alloc::live_bytes();
    let structure = build();
    let after = alloc::live_bytes();
    black_box(traverse(&structure).checksum);
    drop(structure);
    alloc::set_counting(false);
    let warm_up = started.elapsed();
    let predicted = warm_up * (p.reps as u32 - 1);
    if predicted > p.budget {
        return Err(OverBudget {
            predicted,
            budget: p.budget,
        });
    }

    let mut build_ns = Vec::with_capacity(p.reps - 1);
    let mut traverse_ns = Vec::with_capacity(p.reps - 1);
    for _ in 1..p.reps {
        let t = Instant::now();
        let structure = build();
        let built = t.elapsed();
        let t = Instant::now();
        let pass = traverse(black_box(&structure));
        let traversed = t.elapsed();
        black_box(pass.checksum);
        build_ns.push(built.as_nanos() as f64 / nodes as f64);
        traverse_ns.push(traversed.as_nanos() as f64 / pass.units.max(1) as f64);
        drop(structure);
    }
    Ok(RepStats {
        heap_bytes_per_node: after.saturating_sub(before) as f64 / nodes as f64,
        build: dist(&build_ns),
        traverse: dist(&traverse_ns),
    })
}

/// One line of `rlb` output, and one row of `bench/results.jsonl`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Measurement {
    /// Workload name.
    pub workload: String,
    /// Variant name.
    pub variant: String,
    /// Workload size.
    pub n: u64,
    /// Repetitions requested (including the warm-up).
    pub reps: u64,
    /// Seed.
    pub seed: u64,
    /// `"done"`, `"unreachable"`, `"skipped"` or `"failed"`.
    pub status: String,
    /// Why a cell is unreachable, skipped or failed; empty otherwise.
    pub note: String,
    /// Traversal unit (`"lookup"`, `"visit"`, `"edge"`, `"node"`); empty unless done.
    pub unit: String,
    /// `size_of` one node in bytes.
    pub node_bytes: u64,
    /// Live heap bytes requested per node after the build.
    pub heap_bytes_per_node: f64,
    /// Median build ns per node.
    pub build_ns_median: f64,
    /// Build MAD.
    pub build_ns_mad: f64,
    /// Median traversal ns per unit.
    pub traverse_ns_median: f64,
    /// Traversal MAD.
    pub traverse_ns_mad: f64,
    /// Peak resident set size of the process in KiB (`VmHWM`).
    pub peak_rss_kib: u64,
    /// Minor page faults of the process.
    pub minor_faults: u64,
    /// Major page faults of the process.
    pub major_faults: u64,
    /// Number of links measured by the reach statistics.
    pub reach_links: u64,
    /// Fraction of links within distance 127.
    pub reach_within_127: f64,
    /// Fraction of links within distance 32,767.
    pub reach_within_32767: f64,
    /// Largest link distance.
    pub reach_max_abs_delta: u64,
}

impl Measurement {
    /// Builds a measurement from a workload outcome. Process-level fields stay zero.
    pub fn from_outcome(workload: &str, variant: &str, p: &Params, outcome: &Outcome) -> Self {
        let mut m = Measurement {
            workload: workload.to_owned(),
            variant: variant.to_owned(),
            n: p.n as u64,
            reps: p.reps as u64,
            seed: p.seed,
            ..Measurement::default()
        };
        match outcome {
            Outcome::Done(metrics) => {
                m.status = "done".to_owned();
                m.unit = metrics.unit.to_owned();
                m.node_bytes = metrics.node_bytes as u64;
                m.heap_bytes_per_node = metrics.heap_bytes_per_node;
                m.build_ns_median = metrics.build.median;
                m.build_ns_mad = metrics.build.mad;
                m.traverse_ns_median = metrics.traverse.median;
                m.traverse_ns_mad = metrics.traverse.mad;
                if let Some(reach) = &metrics.reach {
                    m.set_reach(reach);
                }
            }
            Outcome::Unreachable(reach) => {
                m.status = "unreachable".to_owned();
                m.note = format!(
                    "links do not fit: max distance {}, {:.1}% within 127, {:.1}% within 32767",
                    reach.max_abs_delta,
                    reach.within_127 * 100.0,
                    reach.within_32767 * 100.0
                );
                m.set_reach(reach);
            }
            Outcome::Skipped(reason) => {
                m.status = "skipped".to_owned();
                m.note = reason.clone();
            }
        }
        m
    }

    fn set_reach(&mut self, reach: &Reach) {
        self.reach_links = reach.links;
        self.reach_within_127 = reach.within_127;
        self.reach_within_32767 = reach.within_32767;
        self.reach_max_abs_delta = reach.max_abs_delta;
    }

    /// Serialises to one line of flat JSON.
    pub fn to_json(&self) -> String {
        let mut obj = JsonObject::default();
        obj.string("workload", &self.workload);
        obj.string("variant", &self.variant);
        obj.number("n", self.n);
        obj.number("reps", self.reps);
        obj.number("seed", self.seed);
        obj.string("status", &self.status);
        obj.string("note", &self.note);
        obj.string("unit", &self.unit);
        obj.number("node_bytes", self.node_bytes);
        obj.float("heap_bytes_per_node", self.heap_bytes_per_node);
        obj.float("build_ns_median", self.build_ns_median);
        obj.float("build_ns_mad", self.build_ns_mad);
        obj.float("traverse_ns_median", self.traverse_ns_median);
        obj.float("traverse_ns_mad", self.traverse_ns_mad);
        obj.number("peak_rss_kib", self.peak_rss_kib);
        obj.number("minor_faults", self.minor_faults);
        obj.number("major_faults", self.major_faults);
        obj.number("reach_links", self.reach_links);
        obj.float("reach_within_127", self.reach_within_127);
        obj.float("reach_within_32767", self.reach_within_32767);
        obj.number("reach_max_abs_delta", self.reach_max_abs_delta);
        obj.finish()
    }

    /// Parses a line produced by [`Measurement::to_json`]. Unknown keys are ignored and missing
    /// keys keep their default.
    pub fn from_json(line: &str) -> Result<Self, String> {
        let fields = parse_flat_object(line)?;
        let text = |key: &str| -> String {
            match fields.iter().find(|(k, _)| k == key) {
                Some((_, Value::Str(s))) => s.clone(),
                Some((_, Value::Num(s))) => s.clone(),
                None => String::new(),
            }
        };
        let int = |key: &str| -> u64 {
            let t = text(key);
            t.parse::<u64>()
                .unwrap_or_else(|_| t.parse::<f64>().map_or(0, |f| f as u64))
        };
        let float = |key: &str| -> f64 { text(key).parse::<f64>().unwrap_or(0.0) };
        Ok(Measurement {
            workload: text("workload"),
            variant: text("variant"),
            n: int("n"),
            reps: int("reps"),
            seed: int("seed"),
            status: text("status"),
            note: text("note"),
            unit: text("unit"),
            node_bytes: int("node_bytes"),
            heap_bytes_per_node: float("heap_bytes_per_node"),
            build_ns_median: float("build_ns_median"),
            build_ns_mad: float("build_ns_mad"),
            traverse_ns_median: float("traverse_ns_median"),
            traverse_ns_mad: float("traverse_ns_mad"),
            peak_rss_kib: int("peak_rss_kib"),
            minor_faults: int("minor_faults"),
            major_faults: int("major_faults"),
            reach_links: int("reach_links"),
            reach_within_127: float("reach_within_127"),
            reach_within_32767: float("reach_within_32767"),
            reach_max_abs_delta: int("reach_max_abs_delta"),
        })
    }
}

#[derive(Default)]
struct JsonObject {
    buf: String,
}

impl JsonObject {
    fn key(&mut self, key: &str) {
        self.buf.push(if self.buf.is_empty() { '{' } else { ',' });
        self.buf.push('"');
        self.buf.push_str(key);
        self.buf.push_str("\":");
    }

    fn string(&mut self, key: &str, value: &str) {
        self.key(key);
        self.buf.push('"');
        for c in value.chars() {
            match c {
                '"' => self.buf.push_str("\\\""),
                '\\' => self.buf.push_str("\\\\"),
                '\n' => self.buf.push_str("\\n"),
                '\r' => self.buf.push_str("\\r"),
                '\t' => self.buf.push_str("\\t"),
                c if (c as u32) < 0x20 => self.buf.push_str(&format!("\\u{:04x}", c as u32)),
                c => self.buf.push(c),
            }
        }
        self.buf.push('"');
    }

    fn number(&mut self, key: &str, value: u64) {
        self.key(key);
        self.buf.push_str(&value.to_string());
    }

    fn float(&mut self, key: &str, value: f64) {
        self.key(key);
        let value = if value.is_finite() { value } else { 0.0 };
        self.buf.push_str(&format!("{value}"));
    }

    fn finish(mut self) -> String {
        self.buf.push('}');
        self.buf
    }
}

enum Value {
    Str(String),
    Num(String),
}

fn parse_flat_object(line: &str) -> Result<Vec<(String, Value)>, String> {
    let mut chars = line.trim().chars().peekable();
    let mut fields = Vec::new();
    let skip_ws = |chars: &mut std::iter::Peekable<std::str::Chars<'_>>| {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
    };
    let parse_string =
        |chars: &mut std::iter::Peekable<std::str::Chars<'_>>| -> Result<String, String> {
            if chars.next() != Some('"') {
                return Err("expected a string".to_owned());
            }
            let mut out = String::new();
            loop {
                match chars.next().ok_or("unterminated string")? {
                    '"' => return Ok(out),
                    '\\' => match chars.next().ok_or("unterminated escape")? {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            let hex: String = chars.by_ref().take(4).collect();
                            let code = u32::from_str_radix(&hex, 16).map_err(|e| e.to_string())?;
                            out.push(char::from_u32(code).ok_or("bad unicode escape")?);
                        }
                        other => return Err(format!("unknown escape \\{other}")),
                    },
                    c => out.push(c),
                }
            }
        };

    skip_ws(&mut chars);
    if chars.next() != Some('{') {
        return Err("expected `{`".to_owned());
    }
    loop {
        skip_ws(&mut chars);
        if chars.peek() == Some(&'}') {
            chars.next();
            return Ok(fields);
        }
        let key = parse_string(&mut chars)?;
        skip_ws(&mut chars);
        if chars.next() != Some(':') {
            return Err("expected `:`".to_owned());
        }
        skip_ws(&mut chars);
        let value = if chars.peek() == Some(&'"') {
            Value::Str(parse_string(&mut chars)?)
        } else {
            let mut token = String::new();
            while chars
                .peek()
                .is_some_and(|c| !matches!(c, ',' | '}') && !c.is_whitespace())
            {
                token.push(chars.next().unwrap_or_default());
            }
            Value::Num(if token == "null" {
                "0".to_owned()
            } else {
                token
            })
        };
        fields.push((key, value));
        skip_ws(&mut chars);
        match chars.next() {
            Some(',') => {}
            Some('}') => return Ok(fields),
            _ => return Err("expected `,` or `}`".to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workload::Metrics;

    fn quick(reps: usize) -> Params {
        Params {
            n: 10,
            reps,
            seed: 1,
            budget: Duration::from_secs(60),
        }
    }

    #[test]
    fn run_reps_builds_once_per_repetition_and_discards_nothing_but_the_warm_up() {
        let mut builds = 0;
        let mut traversals = 0;
        let stats = run_reps(
            &quick(5),
            10,
            || {
                builds += 1;
                vec![1u8; 10]
            },
            |v| {
                traversals += 1;
                Pass {
                    checksum: v.len() as u64,
                    units: 10,
                }
            },
        )
        .unwrap();
        assert_eq!((builds, traversals), (5, 5));
        assert!(stats.build.median >= 0.0 && stats.traverse.median >= 0.0);
        assert_eq!(stats.heap_bytes_per_node, 0.0); // no counting allocator in unit tests
    }

    #[test]
    fn run_reps_skips_a_cell_that_exceeds_its_budget() {
        let params = Params {
            budget: Duration::ZERO,
            ..quick(4)
        };
        let result = run_reps(
            &params,
            1,
            || std::thread::sleep(Duration::from_millis(2)),
            |_| Pass {
                checksum: 0,
                units: 1,
            },
        );
        assert!(matches!(result, Err(OverBudget { .. })));
    }

    #[test]
    #[should_panic(expected = "warm-up")]
    fn run_reps_needs_a_measured_repetition() {
        let _ = run_reps(
            &quick(1),
            1,
            || (),
            |_| Pass {
                checksum: 0,
                units: 1,
            },
        );
    }

    fn sample() -> Measurement {
        Measurement {
            workload: "bst-random".to_owned(),
            variant: "rel16".to_owned(),
            n: 1 << 20,
            reps: 6,
            seed: u64::MAX,
            status: "done".to_owned(),
            note: "quote \" backslash \\ newline \n tab \t".to_owned(),
            unit: "lookup".to_owned(),
            node_bytes: 8,
            heap_bytes_per_node: 8.5,
            build_ns_median: 123.25,
            build_ns_mad: 1.5,
            traverse_ns_median: 99.75,
            traverse_ns_mad: 0.25,
            peak_rss_kib: 123_456,
            minor_faults: 7,
            major_faults: 1,
            reach_links: 42,
            reach_within_127: 0.5,
            reach_within_32767: 0.75,
            reach_max_abs_delta: 99_999,
        }
    }

    #[test]
    fn json_round_trips_every_field() {
        let m = sample();
        assert_eq!(Measurement::from_json(&m.to_json()).unwrap(), m);
    }

    #[test]
    fn json_is_a_single_line() {
        assert!(!sample().to_json().contains('\n'));
    }

    #[test]
    fn json_parser_ignores_unknown_keys_and_rejects_garbage() {
        let m = Measurement::from_json(r#"{"workload":"w","extra":3,"n":5}"#).unwrap();
        assert_eq!((m.workload.as_str(), m.n), ("w", 5));
        assert!(Measurement::from_json("not json").is_err());
        assert!(Measurement::from_json(r#"{"workload":"unterminated"#).is_err());
    }

    #[test]
    fn non_finite_floats_serialise_as_zero() {
        let m = Measurement {
            build_ns_median: f64::NAN,
            ..Measurement::default()
        };
        assert_eq!(
            Measurement::from_json(&m.to_json())
                .unwrap()
                .build_ns_median,
            0.0
        );
    }

    #[test]
    fn outcomes_map_to_statuses() {
        let p = Params::new(100);
        let done = Outcome::Done(Metrics {
            node_bytes: 12,
            heap_bytes_per_node: 12.0,
            build: Dist {
                median: 1.0,
                mad: 0.1,
            },
            traverse: Dist {
                median: 2.0,
                mad: 0.2,
            },
            unit: "lookup",
            reach: Some(Reach::from_deltas([1, 2, 300])),
        });
        let m = Measurement::from_outcome("w", "v", &p, &done);
        assert_eq!(
            (m.status.as_str(), m.unit.as_str(), m.node_bytes),
            ("done", "lookup", 12)
        );
        assert_eq!(
            (m.traverse_ns_median, m.reach_links, m.reach_max_abs_delta),
            (2.0, 3, 300)
        );

        let unreachable = Outcome::Unreachable(Reach::from_deltas([500]));
        let m = Measurement::from_outcome("w", "v", &p, &unreachable);
        assert_eq!(m.status, "unreachable");
        assert!(m.note.contains("max distance 500"));

        let m = Measurement::from_outcome("w", "v", &p, &Outcome::Skipped("too slow".to_owned()));
        assert_eq!(
            (m.status.as_str(), m.note.as_str()),
            ("skipped", "too slow")
        );
    }
}
