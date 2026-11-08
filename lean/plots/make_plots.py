#!/usr/bin/env python3
"""Generate labeled line charts from results.jsonl into index.html (inline SVG)."""
import json
import math
from pathlib import Path


def load(path, workload, variants):
    data = {v: {} for v in variants}
    for line in open(path):
        j = json.loads(line)
        if j["status"] != "done" or j["workload"] != workload or j["variant"] not in data:
            continue
        # node_bytes is size_of; heap_bytes_per_node is requested heap
        data[j["variant"]][j["n"]] = j["traverse_ns_median"]
    return data


def ratio_svg(title, subtitle, pts):
    W, H = 880, 560
    mx, my_base, plot_h, plot_w = 80, 450, 340, 560
    ymin, ymax = 0.3, 1.5
    xmin = 10
    xmax = max(math.log2(n) for n, _ in pts)

    def xpos(e):
        return mx + (e - xmin) / (xmax - xmin) * plot_w

    def ypos(v):
        return my_base - (v - ymin) / (ymax - ymin) * plot_h

    svg = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="100%">',
           '<rect width="100%" height="100%" fill="#ffffff"/>',
           f'<text x="30" y="30" font-size="15" font-weight="600" fill="#111">{title}</text>',
           f'<text x="30" y="50" font-size="12" fill="#555">{subtitle}</text>']

    for t in [0.4, 0.6, 0.8, 1.0, 1.2]:
        y = ypos(t)
        col = "#000" if t == 1.0 else "#e0e0e0"
        w = "2" if t == 1.0 else "1"
        svg.append(f'<line x1="{mx}" y1="{y:.0f}" x2="{mx + plot_w}" y2="{y:.0f}" stroke="{col}" stroke-width="{w}"/>')
        svg.append(f'<text x="{mx - 8}" y="{y + 4:.0f}" font-size="12" text-anchor="end" fill="#111">{t:g}</text>')

    for e in range(int(xmin), int(xmax) + 1, 2):
        x = xpos(e)
        svg.append(f'<text x="{x:.0f}" y="{my_base + 20}" font-size="12" text-anchor="middle" fill="#111">2^{e}</text>')

    svg.append(f'<line x1="{mx}" y1="70" x2="{mx}" y2="{my_base}" stroke="#111" stroke-width="2"/>')
    svg.append(f'<line x1="{mx}" y1="{my_base}" x2="{mx + plot_w}" y2="{my_base}" stroke="#111" stroke-width="2"/>')
    svg.append(f'<text x="{mx + plot_w // 2}" y="{my_base + 45}" font-size="13" text-anchor="middle" fill="#111">n = number of elements (log2)</text>')
    svg.append(f'<text x="22" y="{my_base - plot_h // 2}" font-size="13" fill="#111" '
               f'transform="rotate(-90 22 {my_base - plot_h // 2})" text-anchor="middle">ratio (narrow / baseline)</text>')

    pts_s = " ".join(f"{xpos(math.log2(n)):.0f},{ypos(v):.0f}" for n, v in pts)
    svg.append(f'<polyline points="{pts_s}" fill="none" stroke="#2563eb" stroke-width="2"/>')
    for n, v in pts:
        svg.append(f'<circle cx="{xpos(math.log2(n)):.0f}" cy="{ypos(v):.0f}" r="3" fill="#2563eb"/>')

    # End-of-line value label + legend
    n0, v0 = pts[-1]
    svg.append(f'<line x1="{mx + plot_w + 10}" y1="80" x2="{mx + plot_w + 34}" y2="80" stroke="#2563eb" stroke-width="2"/>')
    svg.append(f'<text x="{mx + plot_w + 42}" y="84" font-size="12" fill="#2563eb">u32/std</text>')
    svg.append(f'<text x="{xpos(math.log2(n0)) + 8:.0f}" y="{ypos(v0) + 4:.0f}" font-size="12" fill="#2563eb">{v0:.2f} @ 2^{int(math.log2(n0))}</text>')
    svg.append("</svg>")
    return "\n".join(svg)


def ratio_svg_ng(title, subtitle, pts):
    # ngraph uses abs32/rc, otherwise same structure
    W, H = 880, 560
    mx, my_base, plot_h, plot_w = 80, 450, 340, 560
    ymin, ymax = 0.3, 1.5
    xmin = 10
    xmax = max(math.log2(n) for n, _ in pts)

    def xpos(e):
        return mx + (e - xmin) / (xmax - xmin) * plot_w

    def ypos(v):
        return my_base - (v - ymin) / (ymax - ymin) * plot_h

    svg = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="100%">',
           '<rect width="100%" height="100%" fill="#ffffff"/>',
           f'<text x="30" y="30" font-size="15" font-weight="600" fill="#111">{title}</text>',
           f'<text x="30" y="50" font-size="12" fill="#555">{subtitle}</text>']
    for t in [0.4, 0.6, 0.8, 1.0, 1.2]:
        y = ypos(t)
        col = "#000" if t == 1.0 else "#e0e0e0"
        w = "2" if t == 1.0 else "1"
        svg.append(f'<line x1="{mx}" y1="{y:.0f}" x2="{mx + plot_w}" y2="{y:.0f}" stroke="{col}" stroke-width="{w}"/>')
        svg.append(f'<text x="{mx - 8}" y="{y + 4:.0f}" font-size="12" text-anchor="end" fill="#111">{t:g}</text>')
    for e in range(int(xmin), int(xmax) + 1, 2):
        x = xpos(e)
        svg.append(f'<text x="{x:.0f}" y="{my_base + 20}" font-size="12" text-anchor="middle" fill="#111">2^{e}</text>')
    svg.append(f'<line x1="{mx}" y1="70" x2="{mx}" y2="{my_base}" stroke="#111" stroke-width="2"/>')
    svg.append(f'<line x1="{mx}" y1="{my_base}" x2="{mx + plot_w}" y2="{my_base}" stroke="#111" stroke-width="2"/>')
    svg.append(f'<text x="{mx + plot_w // 2}" y="{my_base + 45}" font-size="13" text-anchor="middle" fill="#111">n = number of elements (log2)</text>')
    svg.append(f'<text x="22" y="{my_base - plot_h // 2}" font-size="13" fill="#111" '
               f'transform="rotate(-90 22 {my_base - plot_h // 2})" text-anchor="middle">ratio (narrow / baseline)</text>')
    pts_s = " ".join(f"{xpos(math.log2(n)):.0f},{ypos(v):.0f}" for n, v in pts)
    svg.append(f'<polyline points="{pts_s}" fill="none" stroke="#2563eb" stroke-width="2"/>')
    for n, v in pts:
        svg.append(f'<circle cx="{xpos(math.log2(n)):.0f}" cy="{ypos(v):.0f}" r="3" fill="#2563eb"/>')
    n0, v0 = pts[-1]
    svg.append(f'<line x1="{mx + plot_w + 10}" y1="80" x2="{mx + plot_w + 34}" y2="80" stroke="#2563eb" stroke-width="2"/>')
    svg.append(f'<text x="{mx + plot_w + 42}" y="84" font-size="12" fill="#2563eb">abs32/rc</text>')
    svg.append(f'<text x="{xpos(math.log2(n0)) + 8:.0f}" y="{ypos(v0) + 4:.0f}" font-size="12" fill="#2563eb">{v0:.2f} @ 2^{int(math.log2(n0))}</text>')
    svg.append("</svg>")
    return "\n".join(svg)


bt = load("bench/containers/results.jsonl", "btree-lookup", ["std", "u32"])
ng = load("bench/decisive/results.jsonl", "ngraph-mesh", ["rc", "abs32"])
bp = [(n, bt["u32"][n] / bt["std"][n]) for n in sorted(bt["u32"]) if n in bt["std"]]
np_ = [(n, ng["abs32"][n] / ng["rc"][n]) for n in sorted(ng["abs32"]) if n in ng["rc"]]

html = """<!doctype html>
<html><head><meta charset="utf-8"><title>rustlink plots</title>
<style>body{font-family:monospace;max-width:940px;margin:0 auto;padding:20px}svg{width:100%;height:auto;border:1px solid #ccc;margin-bottom:20px}</style>
</head><body>
<h1>rustlink baseline vs narrow (results.jsonl)</h1>
<p>u32 is 0.69× std at 2^20; abs32 is 0.58× rc at 2^22.</p>
"""
html += "<h2>btree-lookup ratio (rustlink u32 / std)</h2>\n" + ratio_svg(
    "btree-lookup ratio u32/std", "bench/containers/results.jsonl", bp) + "\n"
html += "<h2>ngraph-mesh ratio (abs32 / rc)</h2>\n" + ratio_svg_ng(
    "ngraph-mesh ratio abs32/rc", "bench/decisive/results.jsonl", np_) + "\n"
html += "</body></html>"

Path("lean/plots/index.html").write_text(html)
print("wrote lean/plots/index.html")
print(bp)
print(np_)
