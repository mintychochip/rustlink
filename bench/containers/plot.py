#!/usr/bin/env python3
"""Render the container benchmark record as self-contained SVGs. Stdlib only."""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
DATA = ROOT / "results.jsonl"
OUT = ROOT / "plots"

COLORS = {"std": "#8b949e", "std-fnv": "#a371f7", "u16": "#2da44e", "u32": "#2f81f7"}
FG = "#6e7781"
GRID = "#8b949e"


def load():
    rows = [json.loads(line) for line in DATA.read_text().splitlines() if line.strip()]
    return {(r["workload"], r["variant"], r["n"]): r for r in rows}


def nice_max(value):
    import math
    value = max(value, 1e-9)
    exp = math.floor(math.log10(value))
    base = 10.0 ** exp
    for mult in (1.0, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0):
        if value <= mult * base:
            return mult * base
    return 10.0 * base


def esc(text):
    return str(text).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def panel(x, y, title, unit, series, sizes):
    """One line chart. series is [(label, color, {n: value})]."""
    w, h = 280, 180
    left, top, right, bottom = 42, 38, 10, 22
    pw, ph = w - left - right, h - top - bottom
    values = [v for _, _, pts in series for v in pts.values()]
    vmax = nice_max(max(values) * 1.08) if values else 1.0
    out = [f'<g transform="translate({x},{y})">']
    out.append(f'<text x="0" y="13" font-size="10.5" font-weight="600" fill="{FG}">{esc(title)}</text>')
    # legend
    lx = 0
    for label, color, _ in series:
        out.append(f'<line x1="{lx}" y1="24" x2="{lx+12}" y2="24" stroke="{color}" stroke-width="2"/>')
        out.append(f'<text x="{lx+15}" y="27" font-size="8.5" fill="{FG}">{esc(label)}</text>')
        lx += 15 + 6.2 * len(label) + 9
    # grid and y labels
    for frac in (0.0, 0.5, 1.0):
        gy = top + ph * (1 - frac)
        out.append(f'<line x1="{left}" y1="{gy:.1f}" x2="{left+pw}" y2="{gy:.1f}" stroke="{GRID}" stroke-opacity="0.35" stroke-width="0.7"/>')
        out.append(f'<text x="{left-5}" y="{gy+3:.1f}" font-size="8" text-anchor="end" fill="{FG}">{vmax*frac:.3g}</text>')
    out.append(f'<line x1="{left}" y1="{top}" x2="{left}" y2="{top+ph}" stroke="{GRID}" stroke-width="0.8"/>')
    out.append(f'<line x1="{left}" y1="{top+ph}" x2="{left+pw}" y2="{top+ph}" stroke="{GRID}" stroke-width="0.8"/>')
    npos = {n: left + pw * i / (len(sizes) - 1) for i, n in enumerate(sizes)}
    for n, px in npos.items():
        out.append(f'<text x="{px:.1f}" y="{top+ph+13}" font-size="8" text-anchor="middle" fill="{FG}">2^{n.bit_length()-1}</text>')
    for _, color, pts in series:
        points = [(npos[n], top + ph * (1 - v / vmax)) for n, v in pts.items() if n in npos]
        if len(points) > 1:
            path = " ".join(f"{px:.1f},{py:.1f}" for px, py in points)
            out.append(f'<polyline points="{path}" fill="none" stroke="{color}" stroke-width="1.7" stroke-linejoin="round"/>')
        for px, py in points:
            out.append(f'<circle cx="{px:.1f}" cy="{py:.1f}" r="2.1" fill="{color}"/>')
    out.append(f'<text x="{left+pw}" y="{h-2}" font-size="8" text-anchor="end" fill="{FG}">{esc(unit)}</text>')
    out.append('</g>')
    return "".join(out), vmax


def cpu_svg(data):
    sizes = [1 << e for e in (10, 12, 14, 16, 18, 20)]
    panels = [
        ("B-tree lookup", "ns / lookup", [("std", "std"), ("u32", "u32")]),
        ("B-tree scan", "ns / visit", [("std", "std"), ("u32", "u32")]),
        ("B-tree churn", "ns / op", [("std", "std"), ("u32", "u32")]),
        ("Hash lookup", "ns / lookup", [("std", "std"), ("std-fnv", "std-fnv"), ("u32", "u32")]),
        ("Hash scan", "ns / visit", [("std", "std"), ("std-fnv", "std-fnv"), ("u32", "u32")]),
        ("Hash churn", "ns / op", [("std", "std"), ("std-fnv", "std-fnv"), ("u32", "u32")]),
        ("List scan", "ns / visit", [("std", "std"), ("u32", "u32")]),
    ]
    work = {"B-tree lookup": "btree-lookup", "B-tree scan": "btree-scan", "B-tree churn": "btree-churn",
            "Hash lookup": "hash-lookup", "Hash scan": "hash-scan", "Hash churn": "hash-churn",
            "List scan": "list-scan"}
    parts = ['<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 900 620" role="img" font-family="-apple-system,Segoe UI,Helvetica,Arial,sans-serif">',
             f'<text x="20" y="22" font-size="14" font-weight="600" fill="{FG}">Traversal CPU time vs entries</text>',
             f'<text x="20" y="36" font-size="9" fill="{FG}">median of 14 sweeps, cpu23; lower is better; lines are std, std-fnv and u32 where present</text>']
    for i, (title, unit, variants) in enumerate(panels):
        wl = work[title]
        series = []
        for label, variant in variants:
            pts = {n: data[(wl, variant, n)]["traverse_cpu_ns_median"]
                   for n in sizes if (wl, variant, n) in data and data[(wl, variant, n)]["status"] == "done"}
            series.append((label, COLORS[variant], pts))
        part, _ = panel(20 + (i % 3) * 295, 50 + (i // 3) * 188, title, unit, series, sizes)
        parts.append(part)
    parts.append('</svg>')
    return "".join(parts)


def bar_panel(x, y, title, unit, groups, series, vmax):
    w, h = 440, 270
    left, top, right, bottom = 46, 34, 10, 34
    pw, ph = w - left - right, h - top - bottom
    out = [f'<g transform="translate({x},{y})">',
           f'<text x="0" y="13" font-size="10.5" font-weight="600" fill="{FG}">{esc(title)}</text>']
    lx = 0
    for label, color in series:
        out.append(f'<rect x="{lx}" y="19" width="8" height="8" fill="{color}"/>')
        out.append(f'<text x="{lx+11}" y="27" font-size="8.5" fill="{FG}">{esc(label)}</text>')
        lx += 11 + 6.0 * len(label) + 10
    for frac in (0.0, 0.5, 1.0):
        gy = top + ph * (1 - frac)
        out.append(f'<line x1="{left}" y1="{gy:.1f}" x2="{left+pw}" y2="{gy:.1f}" stroke="{GRID}" stroke-opacity="0.35" stroke-width="0.7"/>')
        out.append(f'<text x="{left-5}" y="{gy+3:.1f}" font-size="8" text-anchor="end" fill="{FG}">{vmax*frac:.3g}</text>')
    group_w = pw / len(groups)
    for gi, (gname, vals) in enumerate(groups):
        gx = left + gi * group_w
        bw = group_w / (len(series) + 2.2)
        for si, (label, color) in enumerate(series):
            v = vals.get(label)
            if v is None:
                continue
            bh = ph * v / vmax
            bx = gx + group_w * 0.12 + si * bw
            out.append(f'<rect x="{bx:.1f}" y="{top+ph-bh:.1f}" width="{bw*0.82:.1f}" height="{bh:.1f}" fill="{color}" rx="1"/>')
            out.append(f'<text x="{bx+bw*0.41:.1f}" y="{top+ph-bh-2:.1f}" font-size="7.5" text-anchor="middle" fill="{FG}">{v:.1f}</text>')
        out.append(f'<text x="{gx+group_w/2:.1f}" y="{top+ph+14}" font-size="8.5" text-anchor="middle" fill="{FG}">{esc(gname)}</text>')
    out.append(f'<line x1="{left}" y1="{top+ph}" x2="{left+pw}" y2="{top+ph}" stroke="{GRID}" stroke-width="0.8"/>')
    out.append(f'<text x="{left+pw}" y="{h-2}" font-size="8" text-anchor="end" fill="{FG}">{esc(unit)}</text>')
    out.append('</g>')
    return "".join(out)


def memory_svg(data):
    def heap(workload, variant, n):
        return data[(workload, variant, n)]["heap_bytes_per_node"]

    heap_groups = [
        ("B-tree", {"std": heap("btree-lookup", "std", 1 << 20),
                    "u32": heap("btree-lookup", "u32", 1 << 20),
                    "u16": heap("btree-lookup", "u16", 1 << 20)}),
        ("Hash", {"std": heap("hash-lookup", "std", 1 << 20),
                  "u32": heap("hash-lookup", "u32", 1 << 20),
                  "u16": heap("hash-lookup", "u16", 1 << 14)}),
        ("List", {"std": heap("list-scan", "std", 1 << 20),
                  "u32": heap("list-scan", "u32", 1 << 20),
                  "u16": heap("list-scan", "u16", 1 << 14)}),
    ]
    rss_groups = [
        ("B-tree", {"std": data[("btree-lookup", "std", 1 << 20)]["peak_rss_kib"] / 1024,
                     "u32": data[("btree-lookup", "u32", 1 << 20)]["peak_rss_kib"] / 1024}),
        ("Hash", {"std": data[("hash-lookup", "std", 1 << 20)]["peak_rss_kib"] / 1024,
                   "u32": data[("hash-lookup", "u32", 1 << 20)]["peak_rss_kib"] / 1024}),
        ("List", {"std": data[("list-scan", "std", 1 << 20)]["peak_rss_kib"] / 1024,
                   "u32": data[("list-scan", "u32", 1 << 20)]["peak_rss_kib"] / 1024}),
    ]
    parts = ['<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 900 340" role="img" font-family="-apple-system,Segoe UI,Helvetica,Arial,sans-serif">',
             f'<text x="20" y="22" font-size="14" font-weight="600" fill="{FG}">Heap and peak RSS</text>',
             f'<text x="20" y="36" font-size="9" fill="{FG}">heap: requested bytes per entry at 2^20 (u16 at 2^14 for hash/list); RSS: process peak at 2^20</text>']
    parts.append(bar_panel(20, 50, "Heap bytes per entry", "B / entry", heap_groups,
                           [("std", COLORS["std"]), ("u32", COLORS["u32"]), ("u16", COLORS["u16"])], 40))
    parts.append(bar_panel(460, 50, "Peak RSS at 2^20", "MiB", rss_groups,
                           [("std", COLORS["std"]), ("u32", COLORS["u32"])], 50))
    parts.append('</svg>')
    return "".join(parts)


def main():
    data = load()
    OUT.mkdir(exist_ok=True)
    (OUT / "cpu.svg").write_text(cpu_svg(data))
    (OUT / "memory.svg").write_text(memory_svg(data))
    print(f"wrote {OUT/'cpu.svg'} and {OUT/'memory.svg'}")


if __name__ == "__main__":
    main()
