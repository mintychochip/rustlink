#!/usr/bin/env python3
"""Render the set benchmark record. Reuses the chart primitives from the container record."""
import json
import sys

sys.dont_write_bytecode = True
from pathlib import Path

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT.parent / "containers"))
import plot as base  # noqa: E402


def load():
    rows = [json.loads(line) for line in (ROOT / "results.jsonl").read_text().splitlines() if line.strip()]
    return {(r["workload"], r["variant"], r["n"]): r for r in rows}


def cpu_svg(data):
    sizes = [1 << e for e in (10, 12, 14, 16, 18, 20)]
    panels = [
        ("BTreeSet lookup", "ns / lookup", "btreeset-lookup", ["std", "u32"]),
        ("BTreeSet scan", "ns / visit", "btreeset-scan", ["std", "u32"]),
        ("BTreeSet churn", "ns / op", "btreeset-churn", ["std", "u32"]),
        ("HashSet lookup", "ns / lookup", "hashset-lookup", ["std", "std-fnv", "u32"]),
        ("HashSet scan", "ns / visit", "hashset-scan", ["std", "std-fnv", "u32"]),
        ("HashSet churn", "ns / op", "hashset-churn", ["std", "std-fnv", "u32"]),
    ]
    parts = ['<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 900 460" role="img" font-family="-apple-system,Segoe UI,Helvetica,Arial,sans-serif">',
             f'<text x="20" y="22" font-size="14" font-weight="600" fill="{base.FG}">Set traversal CPU time vs entries</text>',
             f'<text x="20" y="36" font-size="9" fill="{base.FG}">median of 14 sweeps, cpu23; lower is better; explicit insert build on both sides</text>']
    for i, (title, unit, wl, variants) in enumerate(panels):
        series = []
        for variant in variants:
            pts = {n: data[(wl, variant, n)]["traverse_cpu_ns_median"]
                   for n in sizes if (wl, variant, n) in data and data[(wl, variant, n)]["status"] == "done"}
            series.append((variant, base.COLORS[variant], pts))
        part, _ = base.panel(20 + (i % 3) * 295, 50 + (i // 3) * 208, title, unit, series, sizes)
        parts.append(part)
    parts.append('</svg>')
    return "".join(parts)


def memory_svg(data):
    def heap(wl, variant, n):
        return data[(wl, variant, n)]["heap_bytes_per_node"]

    groups = [
        ("BTreeSet", {"std": heap("btreeset-lookup", "std", 1 << 20),
                      "u32": heap("btreeset-lookup", "u32", 1 << 20),
                      "u16": heap("btreeset-lookup", "u16", 1 << 20)}),
        ("HashSet", {"std": heap("hashset-lookup", "std", 1 << 20),
                     "u32": heap("hashset-lookup", "u32", 1 << 20),
                     "u16": heap("hashset-lookup", "u16", 1 << 14)}),
    ]
    rss = [
        ("BTreeSet", {"std": data[("btreeset-lookup", "std", 1 << 20)]["peak_rss_kib"] / 1024,
                      "u32": data[("btreeset-lookup", "u32", 1 << 20)]["peak_rss_kib"] / 1024}),
        ("HashSet", {"std": data[("hashset-lookup", "std", 1 << 20)]["peak_rss_kib"] / 1024,
                     "u32": data[("hashset-lookup", "u32", 1 << 20)]["peak_rss_kib"] / 1024}),
    ]
    parts = ['<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 900 340" role="img" font-family="-apple-system,Segoe UI,Helvetica,Arial,sans-serif">',
             f'<text x="20" y="22" font-size="14" font-weight="600" fill="{base.FG}">Set heap and peak RSS</text>',
             f'<text x="20" y="36" font-size="9" fill="{base.FG}">heap: requested bytes per key at 2^20 (u16 HashSet at 2^14); RSS: process peak at 2^20</text>']
    parts.append(base.bar_panel(20, 50, "Heap bytes per key", "B / key", groups,
                                [("std", base.COLORS["std"]), ("u32", base.COLORS["u32"]), ("u16", base.COLORS["u16"])], 20))
    parts.append(base.bar_panel(460, 50, "Peak RSS at 2^20", "MiB", rss,
                                [("std", base.COLORS["std"]), ("u32", base.COLORS["u32"])], 30))
    parts.append('</svg>')
    return "".join(parts)


def main():
    data = load()
    (ROOT / "plots").mkdir(exist_ok=True)
    (ROOT / "plots" / "cpu.svg").write_text(cpu_svg(data))
    (ROOT / "plots" / "memory.svg").write_text(memory_svg(data))
    print("wrote bench/sets/plots/cpu.svg and memory.svg")


if __name__ == "__main__":
    main()
