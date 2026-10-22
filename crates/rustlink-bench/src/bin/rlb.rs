//! `rlb`: runs one benchmark cell in a fresh process and prints one line of JSON.
//!
//! ```text
//! rlb list
//! rlb run --workload W --variant V --n N [--reps R] [--seed S] [--budget-secs B]
//! ```

use std::collections::HashMap;
use std::process::ExitCode;
use std::time::Duration;

use rustlink_bench::alloc::Counting;
use rustlink_bench::measure::{Measurement, Params};
use rustlink_bench::proc;
use rustlink_bench::registry;
use rustlink_bench::workload::Outcome;

#[global_allocator]
static ALLOC: Counting = Counting;

const USAGE: &str = "usage:\n  rlb list\n  rlb run --workload W --variant V --n N [--reps R] [--seed S] [--budget-secs B]";

#[derive(Debug, PartialEq)]
struct RunArgs {
    workload: String,
    variant: String,
    params: Params,
}

fn parse_flags<'a>(
    args: &'a [String],
    known: &[&str],
) -> Result<HashMap<&'a str, &'a str>, String> {
    let mut flags = HashMap::new();
    let mut iter = args.iter();
    while let Some(flag) = iter.next() {
        let name = flag
            .strip_prefix("--")
            .ok_or_else(|| format!("unexpected argument `{flag}`"))?;
        if !known.contains(&name) {
            return Err(format!("unknown flag `--{name}`"));
        }
        let value = iter
            .next()
            .ok_or_else(|| format!("flag `--{name}` needs a value"))?;
        flags.insert(name, value.as_str());
    }
    Ok(flags)
}

fn number<T: std::str::FromStr>(
    flags: &HashMap<&str, &str>,
    name: &str,
) -> Result<Option<T>, String> {
    flags
        .get(name)
        .map(|value| {
            value
                .parse::<T>()
                .map_err(|_| format!("`--{name}` expects a number, got `{value}`"))
        })
        .transpose()
}

fn parse_run(args: &[String]) -> Result<RunArgs, String> {
    let flags = parse_flags(
        args,
        &["workload", "variant", "n", "reps", "seed", "budget-secs"],
    )?;
    let text = |name: &str| {
        flags
            .get(name)
            .map(|value| (*value).to_owned())
            .ok_or_else(|| format!("missing `--{name}`"))
    };
    let n = number::<usize>(&flags, "n")?.ok_or("missing `--n`")?;
    let mut params = Params::new(n);
    if let Some(reps) = number::<usize>(&flags, "reps")? {
        if reps < 2 {
            return Err("`--reps` must be at least 2 (one warm-up, one measured)".to_owned());
        }
        params.reps = reps;
    }
    if let Some(seed) = number::<u64>(&flags, "seed")? {
        params.seed = seed;
    }
    if let Some(secs) = number::<u64>(&flags, "budget-secs")? {
        params.budget = Duration::from_secs(secs);
    }
    Ok(RunArgs {
        workload: text("workload")?,
        variant: text("variant")?,
        params,
    })
}

fn list() {
    for workload in registry::all() {
        println!(
            "{}\t{}\t{}\t{}",
            workload.name(),
            workload.max_n(),
            workload.footprint_per_n(),
            workload.variants().join(",")
        );
    }
}

fn run_cell(args: &[String]) -> Result<(), String> {
    let RunArgs {
        workload: name,
        variant,
        params,
    } = parse_run(args)?;
    let workload = registry::find(&name).ok_or_else(|| format!("unknown workload `{name}`"))?;
    if !workload.variants().contains(&variant.as_str()) {
        return Err(format!("workload `{name}` has no variant `{variant}`"));
    }
    let outcome = if params.n > workload.max_n() {
        Outcome::Skipped("n exceeds the workload's max_n".to_owned())
    } else {
        workload.run(&variant, &params)?
    };
    let mut measurement = Measurement::from_outcome(&name, &variant, &params, &outcome);
    let stats = proc::read_self().map_err(|e| format!("cannot read /proc: {e}"))?;
    measurement.peak_rss_kib = stats.peak_rss_kib;
    measurement.minor_faults = stats.minor_faults;
    measurement.major_faults = stats.major_faults;
    println!("{}", measurement.to_json());
    Ok(())
}

fn run(args: &[String]) -> Result<(), String> {
    match args.split_first() {
        Some((command, _)) if command == "list" => {
            list();
            Ok(())
        }
        Some((command, rest)) if command == "run" => run_cell(rest),
        _ => Err(USAGE.to_owned()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("rlb: {message}");
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

    #[test]
    fn parses_a_full_command_line() {
        let parsed = parse_run(&args(&[
            "--workload",
            "bst-bulk",
            "--variant",
            "rel16",
            "--n",
            "4096",
            "--reps",
            "9",
            "--seed",
            "77",
            "--budget-secs",
            "5",
        ]))
        .unwrap();
        assert_eq!(parsed.workload, "bst-bulk");
        assert_eq!(parsed.variant, "rel16");
        assert_eq!(parsed.params.n, 4096);
        assert_eq!(parsed.params.reps, 9);
        assert_eq!(parsed.params.seed, 77);
        assert_eq!(parsed.params.budget, Duration::from_secs(5));
    }

    #[test]
    fn defaults_come_from_params_new() {
        let parsed = parse_run(&args(&["--workload", "w", "--variant", "v", "--n", "10"])).unwrap();
        assert_eq!(parsed.params, Params::new(10));
    }

    #[test]
    fn rejects_missing_unknown_and_malformed_flags() {
        assert!(
            parse_run(&args(&["--variant", "v", "--n", "1"]))
                .unwrap_err()
                .contains("--workload")
        );
        assert!(
            parse_run(&args(&["--workload", "w", "--n", "1"]))
                .unwrap_err()
                .contains("--variant")
        );
        assert!(
            parse_run(&args(&["--workload", "w", "--variant", "v"]))
                .unwrap_err()
                .contains("--n")
        );
        assert!(
            parse_run(&args(&[
                "--workload",
                "w",
                "--variant",
                "v",
                "--n",
                "1",
                "--bogus",
                "1"
            ]))
            .unwrap_err()
            .contains("unknown flag")
        );
        assert!(
            parse_run(&args(&["--workload", "w", "--variant", "v", "--n", "many"]))
                .unwrap_err()
                .contains("expects a number")
        );
        assert!(
            parse_run(&args(&["--workload", "w", "--variant", "v", "--n"]))
                .unwrap_err()
                .contains("needs a value")
        );
        assert!(
            parse_run(&args(&["positional"]))
                .unwrap_err()
                .contains("unexpected argument")
        );
        assert!(
            parse_run(&args(&[
                "--workload",
                "w",
                "--variant",
                "v",
                "--n",
                "1",
                "--reps",
                "1"
            ]))
            .unwrap_err()
            .contains("at least 2")
        );
    }

    #[test]
    fn unknown_commands_print_the_usage() {
        assert_eq!(run(&args(&[])).unwrap_err(), USAGE);
        assert_eq!(run(&args(&["frobnicate"])).unwrap_err(), USAGE);
    }

    #[test]
    fn invalid_requests_are_errors_not_measurements() {
        let unknown_workload = args(&["run", "--workload", "nope", "--variant", "v", "--n", "8"]);
        assert!(
            run(&unknown_workload)
                .unwrap_err()
                .contains("unknown workload")
        );
    }
}
