//! Process-level measurements from `/proc` and the CPU clock.

#![allow(unsafe_code)]

use std::fs;
use std::io;

/// Resource usage of the current process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcStats {
    /// Peak resident set size in KiB (`VmHWM`).
    pub peak_rss_kib: u64,
    /// Minor page faults.
    pub minor_faults: u64,
    /// Major page faults.
    pub major_faults: u64,
}

/// The value of `key` in `/proc`-style `Key:   1234 kB` text, in the unit the file uses.
pub fn parse_status_kib(text: &str, key: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        let rest = line.strip_prefix(key)?.strip_prefix(':')?;
        rest.split_whitespace().next()?.parse().ok()
    })
}

/// `(minor, major)` page faults from the text of `/proc/<pid>/stat`.
///
/// The command name is in parentheses and may itself contain spaces and parentheses, so fields
/// are counted after the last `)`: `state ppid pgrp session tty_nr tpgid flags minflt cminflt
/// majflt ...`.
pub fn parse_stat_faults(text: &str) -> Option<(u64, u64)> {
    let after_comm = &text[text.rfind(')')? + 1..];
    let fields: Vec<&str> = after_comm.split_whitespace().collect();
    Some((fields.get(7)?.parse().ok()?, fields.get(9)?.parse().ok()?))
}

/// Reads the current process's peak RSS and page faults.
pub fn read_self() -> io::Result<ProcStats> {
    let status = fs::read_to_string("/proc/self/status")?;
    let stat = fs::read_to_string("/proc/self/stat")?;
    let peak_rss_kib = parse_status_kib(&status, "VmHWM")
        .ok_or_else(|| io::Error::other("VmHWM missing from /proc/self/status"))?;
    let (minor_faults, major_faults) =
        parse_stat_faults(&stat).ok_or_else(|| io::Error::other("cannot parse /proc/self/stat"))?;
    Ok(ProcStats {
        peak_rss_kib,
        minor_faults,
        major_faults,
    })
}

/// `MemAvailable` in KiB, if `/proc/meminfo` reports it.
pub fn mem_available_kib() -> Option<u64> {
    parse_status_kib(&fs::read_to_string("/proc/meminfo").ok()?, "MemAvailable")
}

/// The first field of a `/proc/<pid>/schedstat` line: nanoseconds spent on a CPU.
pub fn parse_schedstat_ns(text: &str) -> Option<u64> {
    text.split_whitespace().next()?.parse().ok()
}

/// Nanoseconds this thread has spent running on a CPU.
///
/// Uses `CLOCK_THREAD_CPUTIME_ID`, which has nanosecond resolution on Linux; falls back to the
/// coarser `/proc/thread-self/schedstat` if the clock fails. `None` when neither is available.
pub fn thread_cpu_ns() -> Option<u64> {
    let mut timespec = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `timespec` is a valid, writable `libc::timespec`, and the thread CPU clock takes
    // no other arguments.
    let result = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut timespec) };
    if result == 0 {
        return Some(timespec.tv_sec as u64 * 1_000_000_000 + timespec.tv_nsec as u64);
    }
    parse_schedstat_ns(&fs::read_to_string("/proc/thread-self/schedstat").ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_values_are_found_by_key() {
        let text = "Name:\tfoo\nVmPeak:\t  999 kB\nVmHWM:\t  1234 kB\nVmRSS:\t 1000 kB\n";
        assert_eq!(parse_status_kib(text, "VmHWM"), Some(1234));
        assert_eq!(parse_status_kib(text, "VmRSS"), Some(1000));
        assert_eq!(parse_status_kib(text, "VmSwap"), None);
        assert_eq!(
            parse_status_kib("MemAvailable:   27000000 kB\n", "MemAvailable"),
            Some(27_000_000)
        );
    }

    #[test]
    fn a_key_prefix_does_not_match_a_longer_key() {
        assert_eq!(parse_status_kib("VmHWMX:\t5 kB\n", "VmHWM"), None);
    }

    #[test]
    fn faults_are_read_after_the_command_name() {
        let line = "1234 (rlb) S 1 1234 1234 0 -1 4194560 77 0 3 0 10 20 0 0 20 0 1 0 100 1000 200";
        assert_eq!(parse_stat_faults(line), Some((77, 3)));
    }

    #[test]
    fn command_names_with_spaces_and_parentheses_are_handled() {
        let line = "42 (my (weird) proc) R 1 42 42 0 -1 0 5 0 6 0 0 0 0 0 20 0 1 0 1 1 1";
        assert_eq!(parse_stat_faults(line), Some((5, 6)));
    }

    #[test]
    fn truncated_stat_lines_are_rejected() {
        assert_eq!(parse_stat_faults("1 (x) S 1 1"), None);
        assert_eq!(parse_stat_faults("no parenthesis"), None);
    }

    #[test]
    fn the_live_process_has_a_nonzero_peak_rss() {
        let stats = read_self().unwrap();
        assert!(stats.peak_rss_kib > 0);
    }

    #[test]
    fn the_thread_clock_advances_during_work() {
        let start = thread_cpu_ns().expect("a CPU clock is available");
        let until = std::time::Instant::now() + std::time::Duration::from_millis(2);
        while std::time::Instant::now() < until {
            std::hint::black_box(0u64.wrapping_add(1));
        }
        let end = thread_cpu_ns().expect("a CPU clock is available");
        assert!(end > start, "CPU time did not advance: {start} -> {end}");
    }

    #[test]
    fn schedstat_takes_the_runtime_field() {
        assert_eq!(parse_schedstat_ns("12345 678 9\n"), Some(12_345));
        assert_eq!(parse_schedstat_ns(""), None);
        assert_eq!(parse_schedstat_ns("nope 1 2"), None);
    }

    #[test]
    fn meminfo_reports_available_memory_on_linux() {
        assert!(mem_available_kib().is_some_and(|kib| kib > 0));
    }
}
