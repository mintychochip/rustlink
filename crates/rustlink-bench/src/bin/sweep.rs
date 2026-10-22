//! `sweep`: runs the whole benchmark matrix, one pinned `rlb` process per cell, appends every
//! measurement to a JSONL file and renders the Markdown report.
//!
//! ```text
//! sweep [--out bench/results.jsonl] [--md bench/results.md] [--workloads a,b] [--variants x,y]
//!       [--sizes 10,12,...] [--big] [--reps 6] [--seed 1] [--budget-secs 120] [--cpu N]
//!       [--dry-run] [--report-only]
//! ```

use std::cmp::Reverse;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use rustlink_bench::measure::Measurement;
use rustlink_bench::proc::mem_available_kib;
use rustlink_bench::registry;
use rustlink_bench::report::{self, MachineInfo};
use rustlink_bench::rng::Rng;

const DEFAULT_EXPONENTS: [u32; 8] = [10, 12, 14, 16, 18, 20, 22, 24];
const USAGE: &str = "usage: sweep [--out FILE] [--md FILE] [--workloads a,b] [--variants x,y] \
[--sizes 10,12,...] [--big] [--reps N] [--seed N] [--budget-secs N] [--cpu N] [--dry-run] [--report-only]";

#[derive(Clone, Debug, PartialEq)]
struct Options {
    out: PathBuf,
    md: PathBuf,
    workloads: Option<Vec<String>>,
    variants: Option<Vec<String>>,
    exponents: Vec<u32>,
    reps: usize,
    seed: u64,
    budget_secs: u64,
    cpu: Option<usize>,
    dry_run: bool,
    report_only: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            out: PathBuf::from("bench/results.jsonl"),
            md: PathBuf::from("bench/results.md"),
            workloads: None,
            variants: None,
            exponents: DEFAULT_EXPONENTS.to_vec(),
            reps: 6,
            seed: 1,
            budget_secs: 120,
            cpu: None,
            dry_run: false,
            report_only: false,
        }
    }
}

fn parse_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect()
}

fn parse_number<T: std::str::FromStr>(flag: &str, text: &str) -> Result<T, String> {
    text.parse()
        .map_err(|_| format!("`{flag}` expects a number, got `{text}`"))
}

/// Size exponents: `--sizes` or the default list, plus 26 with `--big`.
fn parse_sizes(arg: Option<&str>, big: bool) -> Result<Vec<u32>, String> {
    let mut exponents: Vec<u32> = match arg {
        Some(text) => parse_list(text)
            .iter()
            .map(|item| parse_number::<u32>("--sizes", item))
            .collect::<Result<_, _>>()?,
        None => DEFAULT_EXPONENTS.to_vec(),
    };
    if let Some(bad) = exponents.iter().find(|&&e| e > 40) {
        return Err(format!("size exponent {bad} is out of range (at most 40)"));
    }
    if big && !exponents.contains(&26) {
        exponents.push(26);
    }
    Ok(exponents)
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let (mut sizes, mut big) = (None, false);
    let mut iter = args.iter();
    while let Some(flag) = iter.next() {
        let mut value = |flag: &str| {
            iter.next()
                .cloned()
                .ok_or_else(|| format!("`{flag}` needs a value"))
        };
        match flag.as_str() {
            "--big" => big = true,
            "--dry-run" => options.dry_run = true,
            "--report-only" => options.report_only = true,
            "--out" => options.out = PathBuf::from(value(flag)?),
            "--md" => options.md = PathBuf::from(value(flag)?),
            "--workloads" => options.workloads = Some(parse_list(&value(flag)?)),
            "--variants" => options.variants = Some(parse_list(&value(flag)?)),
            "--sizes" => sizes = Some(value(flag)?),
            "--reps" => options.reps = parse_number(flag, &value(flag)?)?,
            "--seed" => options.seed = parse_number(flag, &value(flag)?)?,
            "--budget-secs" => options.budget_secs = parse_number(flag, &value(flag)?)?,
            "--cpu" => options.cpu = Some(parse_number(flag, &value(flag)?)?),
            other => return Err(format!("unknown argument `{other}`\n{USAGE}")),
        }
    }
    if options.reps < 2 {
        return Err("`--reps` must be at least 2 (one warm-up, one measured)".to_owned());
    }
    options.exponents = parse_sizes(sizes.as_deref(), big)?;
    Ok(options)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Cell {
    workload: String,
    variant: String,
    exponent: u32,
}

#[derive(Clone, Debug)]
struct WorkloadInfo {
    name: String,
    max_n: usize,
    footprint_per_n: usize,
    variants: Vec<String>,
}

fn skipped(cell: &Cell, options: &Options, note: String) -> Measurement {
    Measurement {
        workload: cell.workload.clone(),
        variant: cell.variant.clone(),
        n: 1u64 << cell.exponent,
        reps: options.reps as u64,
        seed: options.seed,
        status: "skipped".to_owned(),
        note,
        ..Measurement::default()
    }
}

fn failed(cell: &Cell, note: String) -> Measurement {
    Measurement {
        workload: cell.workload.clone(),
        variant: cell.variant.clone(),
        n: 1u64 << cell.exponent,
        status: "failed".to_owned(),
        note,
        ..Measurement::default()
    }
}

/// The cells to run, and the cells skipped up front because they would not fit in memory.
/// Sizes above a workload's `max_n` are left out entirely.
fn plan_cells(
    infos: &[WorkloadInfo],
    options: &Options,
    available_kib: Option<u64>,
) -> (Vec<Cell>, Vec<Measurement>) {
    let (mut cells, mut skips) = (Vec::new(), Vec::new());
    for info in infos {
        if options
            .workloads
            .as_ref()
            .is_some_and(|only| !only.contains(&info.name))
        {
            continue;
        }
        for variant in &info.variants {
            if options
                .variants
                .as_ref()
                .is_some_and(|only| !only.contains(variant))
            {
                continue;
            }
            for &exponent in &options.exponents {
                let n = 1usize << exponent;
                if n > info.max_n {
                    continue;
                }
                let cell = Cell {
                    workload: info.name.clone(),
                    variant: variant.clone(),
                    exponent,
                };
                if let Some(available) = available_kib {
                    let needed = (n as u64).saturating_mul(info.footprint_per_n as u64) / 1024;
                    if needed > available / 2 {
                        let note = format!(
                            "estimated footprint {needed} KiB exceeds half of MemAvailable ({available} KiB)"
                        );
                        skips.push(skipped(&cell, options, note));
                        continue;
                    }
                }
                cells.push(cell);
            }
        }
    }
    (cells, skips)
}

fn shuffle_cells(cells: &mut [Cell], seed: u64) {
    Rng::new(seed).shuffle(cells);
}

/// The CPU with the highest maximum frequency (a performance core on a hybrid part); on ties the
/// lowest-numbered one.
fn pick_cpu(freqs: &[(usize, u64)]) -> Option<usize> {
    freqs
        .iter()
        .max_by_key(|&&(cpu, freq)| (freq, Reverse(cpu)))
        .map(|&(cpu, _)| cpu)
}

fn read_cpu_freqs() -> Vec<(usize, u64)> {
    let Ok(entries) = fs::read_dir("/sys/devices/system/cpu") else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let cpu: usize = name.strip_prefix("cpu")?.parse().ok()?;
            let freq = fs::read_to_string(entry.path().join("cpufreq/cpuinfo_max_freq")).ok()?;
            Some((cpu, freq.trim().parse().ok()?))
        })
        .collect()
}

fn locate_rlb() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate this executable: {e}"))?;
    let rlb = exe.with_file_name("rlb");
    if rlb.exists() {
        Ok(rlb)
    } else {
        Err(format!(
            "{} not found; build it with `cargo build --release --bins`",
            rlb.display()
        ))
    }
}

fn taskset_available() -> bool {
    Command::new("taskset").arg("--version").output().is_ok()
}

fn run_cell(rlb: &Path, cpu: usize, pin: bool, cell: &Cell, options: &Options) -> Measurement {
    let mut command = if pin {
        let mut command = Command::new("taskset");
        command.arg("-c").arg(cpu.to_string()).arg(rlb);
        command
    } else {
        Command::new(rlb)
    };
    command
        .args([
            "run",
            "--workload",
            &cell.workload,
            "--variant",
            &cell.variant,
        ])
        .args(["--n", &(1usize << cell.exponent).to_string()])
        .args([
            "--reps",
            &options.reps.to_string(),
            "--seed",
            &options.seed.to_string(),
        ])
        .args(["--budget-secs", &options.budget_secs.to_string()]);
    match command.output() {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            match stdout.lines().find(|line| !line.trim().is_empty()) {
                Some(line) => Measurement::from_json(line)
                    .unwrap_or_else(|e| failed(cell, format!("unparseable output: {e}"))),
                None => failed(cell, "rlb printed nothing".to_owned()),
            }
        }
        Ok(output) => failed(
            cell,
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ),
        Err(e) => failed(cell, format!("cannot start rlb: {e}")),
    }
}

fn read_results(path: &Path) -> Result<Vec<Measurement>, String> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut measurements = Vec::new();
    for (index, line) in text
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
    {
        match Measurement::from_json(line) {
            Ok(m) => measurements.push(m),
            Err(e) => eprintln!(
                "sweep: {}:{}: skipping unparsable line: {e}",
                path.display(),
                index + 1
            ),
        }
    }
    Ok(measurements)
}

fn write_report(options: &Options, cpu: usize) -> Result<(), String> {
    let measurements = read_results(&options.out)?;
    let text = report::render(&measurements, &MachineInfo::collect(cpu));
    if let Some(parent) = options.md.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    fs::write(&options.md, text)
        .map_err(|e| format!("cannot write {}: {e}", options.md.display()))?;
    eprintln!("sweep: wrote {}", options.md.display());
    Ok(())
}

fn run(args: &[String]) -> Result<(), String> {
    let options = parse_options(args)?;
    let cpu = options.cpu.unwrap_or_else(|| {
        pick_cpu(&read_cpu_freqs()).unwrap_or_else(|| {
            eprintln!("sweep: warning: cannot read cpuinfo_max_freq; using cpu0");
            0
        })
    });
    if options.report_only {
        return write_report(&options, cpu);
    }

    let infos: Vec<WorkloadInfo> = registry::all()
        .iter()
        .map(|w| WorkloadInfo {
            name: w.name().to_owned(),
            max_n: w.max_n(),
            footprint_per_n: w.footprint_per_n(),
            variants: w.variants().iter().map(|v| (*v).to_owned()).collect(),
        })
        .collect();
    let (mut cells, skips) = plan_cells(&infos, &options, mem_available_kib());
    shuffle_cells(&mut cells, options.seed);

    if options.dry_run {
        println!("cpu: {cpu}");
        for cell in &cells {
            println!(
                "run  {} {} 2^{}",
                cell.workload, cell.variant, cell.exponent
            );
        }
        for skip in &skips {
            println!(
                "skip {} {} n={}: {}",
                skip.workload, skip.variant, skip.n, skip.note
            );
        }
        return Ok(());
    }

    let rlb = locate_rlb()?;
    let pin = taskset_available();
    if !pin {
        eprintln!("sweep: warning: `taskset` not found; running unpinned");
    }
    if let Some(parent) = options.out.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let mut results = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&options.out)
        .map_err(|e| format!("cannot open {}: {e}", options.out.display()))?;
    let mut record = |m: &Measurement| -> Result<(), String> {
        writeln!(results, "{}", m.to_json())
            .and_then(|()| results.flush())
            .map_err(|e| format!("cannot write {}: {e}", options.out.display()))
    };
    for skip in &skips {
        record(skip)?;
    }
    for (index, cell) in cells.iter().enumerate() {
        eprint!(
            "sweep: [{}/{}] {} {} 2^{} ... ",
            index + 1,
            cells.len(),
            cell.workload,
            cell.variant,
            cell.exponent
        );
        let measurement = run_cell(&rlb, cpu, pin, cell, &options);
        eprintln!("{}", measurement.status);
        record(&measurement)?;
    }
    write_report(&options, cpu)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("sweep: {message}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| (*s).to_owned()).collect()
    }

    fn info(name: &str, max_n: usize, footprint: usize, variants: &[&str]) -> WorkloadInfo {
        WorkloadInfo {
            name: name.to_owned(),
            max_n,
            footprint_per_n: footprint,
            variants: variants.iter().map(|v| (*v).to_owned()).collect(),
        }
    }

    #[test]
    fn defaults() {
        let options = parse_options(&[]).unwrap();
        assert_eq!(options, Options::default());
        assert_eq!(options.exponents, [10, 12, 14, 16, 18, 20, 22, 24]);
        assert_eq!(
            (options.reps, options.seed, options.budget_secs),
            (6, 1, 120)
        );
    }

    #[test]
    fn parses_every_flag() {
        let options = parse_options(&args(&[
            "--out",
            "a.jsonl",
            "--md",
            "a.md",
            "--workloads",
            "ast,bst-bulk",
            "--variants",
            "u32, rel8",
            "--sizes",
            "10,12",
            "--reps",
            "8",
            "--seed",
            "9",
            "--budget-secs",
            "30",
            "--cpu",
            "4",
            "--dry-run",
            "--report-only",
        ]))
        .unwrap();
        assert_eq!(options.out, PathBuf::from("a.jsonl"));
        assert_eq!(options.md, PathBuf::from("a.md"));
        assert_eq!(
            options.workloads,
            Some(vec!["ast".to_owned(), "bst-bulk".to_owned()])
        );
        assert_eq!(
            options.variants,
            Some(vec!["u32".to_owned(), "rel8".to_owned()])
        );
        assert_eq!(options.exponents, [10, 12]);
        assert_eq!(
            (options.reps, options.seed, options.budget_secs, options.cpu),
            (8, 9, 30, Some(4))
        );
        assert!(options.dry_run && options.report_only);
    }

    #[test]
    fn big_adds_exponent_26_once() {
        assert_eq!(parse_sizes(None, true).unwrap().last(), Some(&26));
        assert_eq!(parse_sizes(Some("20,26"), true).unwrap(), [20, 26]);
        assert_eq!(parse_sizes(Some("10"), false).unwrap(), [10]);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse_options(&args(&["--nope"])).is_err());
        assert!(
            parse_options(&args(&["--reps"]))
                .unwrap_err()
                .contains("needs a value")
        );
        assert!(
            parse_options(&args(&["--reps", "x"]))
                .unwrap_err()
                .contains("expects a number")
        );
        assert!(
            parse_options(&args(&["--reps", "1"]))
                .unwrap_err()
                .contains("at least 2")
        );
        assert!(
            parse_options(&args(&["--sizes", "10,99"]))
                .unwrap_err()
                .contains("out of range")
        );
        assert!(parse_options(&args(&["--sizes", "ten"])).is_err());
    }

    #[test]
    fn cells_follow_the_workload_variant_and_size_filters() {
        let infos = [
            info("a", 1 << 24, 64, &["x", "y"]),
            info("b", 1 << 12, 64, &["x"]),
        ];
        let options = Options {
            exponents: vec![10, 12, 14],
            ..Options::default()
        };
        let (cells, skips) = plan_cells(&infos, &options, None);
        assert!(skips.is_empty());
        let names: Vec<String> = cells
            .iter()
            .map(|c| format!("{}/{}/{}", c.workload, c.variant, c.exponent))
            .collect();
        // Workload b has max_n 2^12, so its 2^14 cell is dropped.
        assert_eq!(
            names,
            [
                "a/x/10", "a/x/12", "a/x/14", "a/y/10", "a/y/12", "a/y/14", "b/x/10", "b/x/12"
            ]
        );

        let only = Options {
            workloads: Some(vec!["a".to_owned()]),
            variants: Some(vec!["y".to_owned()]),
            exponents: vec![10],
            ..Options::default()
        };
        let (cells, _) = plan_cells(&infos, &only, None);
        assert_eq!(cells.len(), 1);
        assert_eq!(
            (cells[0].workload.as_str(), cells[0].variant.as_str()),
            ("a", "y")
        );
    }

    #[test]
    fn cells_that_would_not_fit_in_half_of_available_memory_are_recorded_as_skipped() {
        let infos = [info("big", 1 << 24, 1024, &["v"])];
        let options = Options {
            exponents: vec![10, 20],
            ..Options::default()
        };
        // 2^10 * 1 KiB = 1 MiB fits in half of 4 MiB; 2^20 * 1 KiB = 1 GiB does not.
        let (cells, skips) = plan_cells(&infos, &options, Some(4 * 1024));
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].exponent, 10);
        assert_eq!(skips.len(), 1);
        assert_eq!((skips[0].n, skips[0].status.as_str()), (1 << 20, "skipped"));
        assert!(skips[0].note.contains("exceeds half of MemAvailable"));
    }

    #[test]
    fn shuffling_is_deterministic_and_a_permutation() {
        let make = || {
            (0..20)
                .map(|i| Cell {
                    workload: "w".to_owned(),
                    variant: "v".to_owned(),
                    exponent: i,
                })
                .collect::<Vec<_>>()
        };
        let (mut a, mut b, mut c) = (make(), make(), make());
        shuffle_cells(&mut a, 7);
        shuffle_cells(&mut b, 7);
        shuffle_cells(&mut c, 8);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, make());
        a.sort_by_key(|cell| cell.exponent);
        assert_eq!(a, make());
    }

    #[test]
    fn the_fastest_cpu_is_picked_with_ties_going_to_the_lowest_number() {
        assert_eq!(
            pick_cpu(&[
                (0, 4_000_000),
                (1, 5_500_000),
                (2, 5_500_000),
                (3, 4_000_000)
            ]),
            Some(1)
        );
        assert_eq!(pick_cpu(&[(5, 1)]), Some(5));
        assert_eq!(pick_cpu(&[]), None);
    }

    #[test]
    fn failed_cells_carry_their_note() {
        let cell = Cell {
            workload: "w".to_owned(),
            variant: "v".to_owned(),
            exponent: 10,
        };
        let m = failed(&cell, "boom".to_owned());
        assert_eq!(
            (m.status.as_str(), m.note.as_str(), m.n),
            ("failed", "boom", 1024)
        );
    }
}
