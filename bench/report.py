#!/usr/bin/env python3

import argparse
import html
import json
import math
import os
import random
import re
import sys
import tempfile
from collections import defaultdict
from pathlib import Path

TREATMENT = "hothash"
HASHES = ["hothash", "foldhash", "fxhash", "xxh3", "rapidhash", "gxhash", "gxhash256", "wyhash", "ahash"]
LABELS = {"hothash": "hothash", "foldhash": "foldhash", "fxhash": "FxHash", "xxh3": "xxh3", "rapidhash": "rapidhash",
          "gxhash": "gxhash", "gxhash256": "gxhash-256", "wyhash": "wyhash", "ahash": "aHash"}
CARD_SIZES = [8, 32, 256, 4096, 32 << 30]
L1_MAX = 31 << 10
BLOCK = 10
ITERS = 500
SEED = 7
PREEMPT_NS, PREEMPT_SHARE = 10_000, 0.001

COL = {"control": "#2f6fdb", "treatment": "#e8642c", "good": "#1a7f37", "bad": "#cf222e",
       "grid": "#e6e8eb", "axis": "#8c959f", "text": "#1f2328", "muted": "#656d76"}
MONO = "ui-monospace,SFMono-Regular,Menlo,Consolas,monospace"

def load(path, warmup):
    data = defaultdict(dict)
    dropped = defaultdict(lambda: [0, 0])
    with open(path) as f:
        for line in f:
            r = json.loads(line)
            if r["round"] < warmup:
                continue
            count = dropped[r["hash"]]
            count[1] += 1
            if r["wall_ns"] - r["cpu_ns"] > max(PREEMPT_NS, r["wall_ns"] * PREEMPT_SHARE):
                count[0] += 1
                continue
            data[(r["size"], r["hash"])][r["round"]] = r["ns"]
    return data, dict(dropped)

def median(s):
    if not s:
        return float("nan")
    n = len(s)
    return s[n // 2] if n % 2 else (s[n // 2 - 1] + s[n // 2]) / 2

def quantile(s, q):
    return s[int(q * (len(s) - 1))]

def analyse(data):
    rng = random.Random(SEED)
    sizes = sorted({s for s, h in data if h == TREATMENT})
    rows = {}
    for size in sizes:
        series = {h: data.get((size, h), {}) for h in HASHES}
        rounds = sorted(set.intersection(*(set(v) for v in series.values())))
        if not rounds:
            continue
        p50 = {h: median(sorted(series[h][r] for r in rounds)) for h in HASHES}
        best = min((h for h in HASHES if h != TREATMENT), key=lambda h: p50[h])
        blocks = [rounds[i:i + BLOCK] for i in range(0, len(rounds), BLOCK)]
        boot = []
        for _ in range(ITERS):
            chosen = [r for _ in blocks for r in rng.choice(blocks)]
            t = median(sorted(series[TREATMENT][r] for r in chosen))
            c = median(sorted(series[best][r] for r in chosen))
            boot.append(t / c)
        boot.sort()
        change = ((p50[TREATMENT] / p50[best] - 1) * 100,
                  (quantile(boot, 0.025) - 1) * 100, (quantile(boot, 0.975) - 1) * 100)
        rows[size] = {"p50": p50, "best": best, "change": change, "units": len(rounds), "blocks": len(blocks)}
    return rows

def fmt_num(v):
    if v >= 100_000:
        return f"{v / 1000:,.0f}k"
    if v >= 10_000:
        return f"{v / 1000:,.1f}k"
    if v >= 100:
        return f"{v:,.0f}"
    if v >= 10:
        return f"{v:.1f}"
    return f"{v:.2f}"

def fmt_lat(ns, unit=False):
    if not math.isfinite(ns):
        return "–"
    if ns >= 1e9:
        return f"{fmt_num(ns / 1e9)} s"
    if ns >= 1e6:
        return f"{fmt_num(ns / 1e6)} ms"
    if ns >= 1e3:
        return f"{fmt_num(ns / 1e3)} µs"
    return fmt_num(ns) + (" ns" if unit else "")

def fmt_pair(a, b):
    scale, unit = next(((d, u) for d, u in ((1e9, "s"), (1e6, "ms"), (1e3, "µs")) if max(a, b) >= d), (1, "ns"))
    return f"{fmt_num(a / scale)} → {fmt_num(b / scale)} {unit}"

def ordinal(n):
    return f"{n}{'th' if 10 <= n % 100 <= 20 else {1: 'st', 2: 'nd', 3: 'rd'}.get(n % 10, 'th')}"

def plural(n, word):
    return f"{n} {word}{'' if n == 1 else 's'}"

def fmt_pct(v):
    return "–" if not math.isfinite(v) else f"{v:+.0f}%".replace("-", "−")

def fmt_size(s):
    for unit, shift in (("GiB", 30), ("MiB", 20), ("KiB", 10)):
        if s >= 1 << shift and s % (1 << shift) == 0:
            return f"{s >> shift} {unit}"
    return f"{s} B"

def tone(v):
    if not math.isfinite(v) or abs(v) < 0.5:
        return COL["muted"]
    return COL["good"] if v < 0 else COL["bad"]

def esc(s):
    return html.escape(str(s))

def card(label, value, color, lines):
    body = "".join(f'<div class="sub">{l}</div>' for l in lines)
    return (f'<div class="card"><div class="label">{esc(label)}</div>'
            f'<div class="big" style="color:{color}">{esc(value)}</div>{body}</div>')

def axis_range(rows):
    vals = [v for r in rows.values() for v in r["change"] if math.isfinite(v)]
    lo = min(min(vals), 0)
    hi = min(max(max(vals), 0), 60)
    span = hi - lo
    step = 20 if span > 80 else 10
    return min(math.floor(lo / step) * step, -step), max(math.ceil(hi / step) * step, step), step

W, PAD = 170, 8

def xpos(v, lo, hi):
    return PAD + (min(max(v, lo), hi) - lo) / (hi - lo) * (W - 2 * PAD)

def axis_svg(lo, hi):
    zero = xpos(0, lo, hi)
    parts = [f'<text x="{zero:.1f}" y="11" text-anchor="middle">0</text>']
    if zero >= 40:
        parts.append(f'<text x="0" y="11" text-anchor="start">{fmt_pct(lo)}</text>')
    if W - zero >= 40:
        parts.append(f'<text x="{W}" y="11" text-anchor="end">{fmt_pct(hi)}</text>')
    return f'<svg width="{W}" height="14" viewBox="0 0 {W} 14">{"".join(parts)}</svg>'

def interval_svg(change, lo, hi, step):
    p, l, h = change
    color = tone(p)
    parts = []
    t = lo
    while t <= hi + 1e-9:
        x = xpos(t, lo, hi)
        dash = ' stroke-dasharray="3 3"' if t == 0 else ""
        stroke = COL["axis"] if t == 0 else COL["grid"]
        parts.append(f'<line x1="{x:.1f}" x2="{x:.1f}" y1="0" y2="18" stroke="{stroke}"{dash}/>')
        t += step
    parts.append(f'<line x1="{xpos(l, lo, hi):.1f}" x2="{xpos(h, lo, hi):.1f}" y1="9" y2="9" stroke="{color}" '
                 f'stroke-opacity="0.55" stroke-width="2"/>')
    if p > hi:
        x = xpos(hi, lo, hi)
        parts.append(f'<path d="M{x - 6:.1f},4 L{x:.1f},9 L{x - 6:.1f},14 Z" fill="{color}"/>')
    else:
        parts.append(f'<circle cx="{xpos(p, lo, hi):.1f}" cy="9" r="4.5" fill="{color}"/>')
    return f'<svg width="{W}" height="18" viewBox="0 0 {W} 18">{"".join(parts)}</svg>'

def table(rows):
    lo, hi, step = axis_range(rows)
    head = "".join(f'<th{" class=t" if h == TREATMENT else ""}>{esc(LABELS[h])}</th>' for h in HASHES)
    out = [f'<table><thead><tr><th>Input</th>{head}<th class="iv">{axis_svg(lo, hi)}</th>'
           f'<th colspan="2" class="vs">hothash vs fastest other</th></tr></thead><tbody>']
    for size, r in rows.items():
        fastest = min(r["p50"].values())
        mark = "†" if size > L1_MAX else ""
        cells = []
        for h in HASHES:
            v = r["p50"][h]
            cls = "best" if v == fastest else "near" if v <= fastest * 1.05 else ""
            cells.append(f'<td class="{cls}">{fmt_lat(v)}</td>')
        p, l, h_ = r["change"]
        out.append(f'<tr><td class="size">{fmt_size(size)}{mark}</td>{"".join(cells)}'
                   f'<td class="iv">{interval_svg(r["change"], lo, hi, step)}</td>'
                   f'<td class="pct" style="color:{tone(p)}">{fmt_pct(p)}</td>'
                   f'<td class="who">{esc(LABELS[r["best"]])} · {fmt_pct(l)} to {fmt_pct(h_)}</td></tr>')
    out.append("</tbody></table>")
    return "".join(out)

def environment(meta, dropped):
    f = meta.get("freq_mhz", {})
    secs = meta.get("seconds") or 1
    clock = f"clock p50 {f['p50'] / 1000:.2f} GHz (p1 {f['p1'] / 1000:.2f}, p99 {f['p99'] / 1000:.2f})" if f.get("p50") else "clock not recorded"
    boost = "boost on" if meta.get("boost") == "1" else "boost off"
    load = (f"{meta['irqs'] / secs:,.0f} interrupts/s ({meta['timer_irqs'] / secs:,.0f}/s the tick), "
            f"{meta['involuntary_switches']:,} preemptions in {secs:.0f} s") if "irqs" in meta else "interrupts not recorded"
    total = sum(t for _, t in dropped.values()) or 1
    drop = sum(d for d, _ in dropped.values())
    drop_note = (f"{drop:,} of {total:,} batches ({drop / total * 100:.2f}%) dropped: the thread lost the core for "
                 f"over {PREEMPT_NS // 1000} µs and {PREEMPT_SHARE:.1%} of the batch")
    if meta.get("isolated"):
        where = "isolated core (SMT sibling offline; no other task, movable IRQ or unbound kernel thread on it)"
    else:
        shared = ", ".join(meta.get("cgroups_on_cpu", [])) or "unknown"
        sibling = "offline" if meta.get("smt_sibling_offline") else "online"
        where = f"pinned, not isolated (SMT sibling {sibling}; {shared} could run there)"
    return (f"CPU {meta.get('cpu', '?')} of an {meta.get('cpu_model', '?')}, {where}; governor "
            f"{meta.get('governor', '?')}, {boost}, {clock}; {load}; {drop_note}.")

def rates(rows, h):
    def rate(a, b):
        if a in rows and b in rows:
            return (b - a) / (rows[b]["p50"][h] - rows[a]["p50"][h])
        return float("nan")
    return rate(4 << 10, 16 << 10), rate(64 << 20, 32 << 30), rate(64 << 20, 1 << 30), rate(1 << 30, 32 << 30)

def build(rows, meta, warmup, dropped):
    big_from, big_every = meta.get("big_from", math.inf), meta.get("big_every", 1)
    small = [r for s, r in rows.items() if s < big_from]
    big = [r for s, r in rows.items() if s >= big_from]
    units, blocks = min(r["units"] for r in small), min(r["blocks"] for r in small)
    big_note = (f"; inputs from {fmt_size(big_from)} only every {ordinal(big_every)} round, "
                f"{plural(min(r['units'] for r in big), 'sample')} in {plural(min(r['blocks'] for r in big), 'block')}"
                if big and big_every > 1 else "")
    few = min(r["blocks"] for r in rows.values()) < 10
    wins = sum(1 for r in rows.values() if r["change"][0] < 0)
    sure = sum(1 for r in rows.values() if r["change"][2] < 0)
    cards = []
    for size in CARD_SIZES:
        if size in rows:
            r = rows[size]
            p, l, h = r["change"]
            cards.append(card(f"P50 · {fmt_size(size).upper()}", fmt_pct(p), tone(p),
                              [f'{esc(LABELS[r["best"]])} {fmt_pair(r["p50"][r["best"]], r["p50"][TREATMENT])}',
                               f"95% range {fmt_pct(l)} to {fmt_pct(h)}"]))
    losses = ", ".join(fmt_size(s) for s, r in rows.items() if r["change"][0] >= 0)
    l1, l2, near, far = rates(rows, TREATMENT)
    others = [rates(rows, h)[:2] for h in HASHES if h != TREATMENT]
    kept = max(abs(b / a - 1) * 100 for a, b in others)
    hashes = ", ".join(LABELS[h] for h in HASHES if h != TREATMENT)
    out = [
        f"<h1>hothash vs 8 Rust hashes: latency, 1 B to 32 GiB</h1>",
        f'<section><div class="cards">{"".join(cards)}</div>'
        f'<p class="note">hothash is the fastest at {wins} of {len(rows)} input sizes ({sure} with the whole '
        f'95% range below zero{"; slower at " + losses if losses else ""}). Median time per hash on CPU {meta.get("cpu", "?")} of an {esc(meta.get("cpu_model", "?"))} '
        f'({"isolated" if meta.get("isolated") else "pinned, <b>not isolated</b>"}), each input\'s address taken '
        f'from the previous hash so calls cannot overlap, over {units} rounds after a {warmup}-round warm-up '
        f'({blocks} blocks of {BLOCK}{big_note}). Changes are hothash against the fastest of {esc(hashes)} at each size, '
        f'so negative is faster; ranges are 95% block-bootstrap intervals over rounds'
        f'{" (<b>under 10 blocks for some sizes: those ranges are not meaningful</b>)" if few else ""}.</p></section>',
        f'<section><h2>Latency per input size</h2>'
        f'<div class="hint">p50 per hash, ns unless marked; <b>bold</b> is the fastest, tinted within 5% of it. '
        f'Interval column: dot is hothash vs the fastest other, whisker the 95% range, left of zero is faster.</div>'
        f'<div class="scroll">{table(rows)}</div>'
        f'<p class="foot">† Longer than L1d: read through the L2 path (see notes).</p></section>',
    ]
    notes = [
        f"Inputs: random bytes, 32 KiB of physical memory mapped again and again behind one virtual range. "
        f"Up to 16 KiB everything is read from L1d. Longer inputs cannot stay there, and a plain buffer of "
        f"256 MiB or more is DRAM-bound (every hash then runs at the same ~55 GB/s). Through the repeated "
        f"mappings they read at plain-L2 speed at every length instead: on this CPU, reading the same memory "
        f"through another address takes a slower path in the L1 cache. That cap binds hothash alone: "
        f"{l1:,.0f} GB/s within L1d, {l2:,.0f} GB/s from 64 MiB to 32 GiB; every other hash keeps its L1d rate "
        f"there to within {kept:.0f}%. Beyond 1 GiB hothash loses {(1 - far / near) * 100:.0f}% more per byte "
        f"(page walks, likely: the page tables of 32 GiB take 64 MiB, more than L3).",
        "Calls: the seed is fixed and per-seed state built once, as in a hash table; each hash uses its fastest "
        "one-shot API for a byte slice: foldhash fast (hashbrown's default) and aHash (AES) through "
        "BuildHasher, write and finish; FxHash rustc-hash 2 FxHasher::with_seed; xxh3 with its secret derived "
        "from the seed once (the seeded call re-derives it over 240 B); rapidhash v3 with RapidSecrets; wyhash "
        "final3 with make_secret; gxhash 128-bit, and gxhash-256 its hybrid build (256-bit VAES bulk loop, "
        "nightly only). The chain adds one and (about one cycle) per call.",
        f"Batches: 8192 hashes up to 2 KiB, 16 MiB of input from there, one call from 16 MiB. Every round "
        f"runs all hashes, rotating their order, so clock drift hits all alike, and every size up to 64 MiB; "
        f"from 256 MiB, where one round of all hashes takes about 8 s, every {ordinal(big_every)} round.",
        f"Environment: {esc(environment(meta, dropped))}",
        f"Build: {esc(meta.get('rustc', '?'))}, {esc(meta.get('rustflags', '?'))}, fat LTO, one codegen unit; "
        f"{esc(meta.get('crates', '?'))}.",
        "Speed only, one machine. hothash needs AVX-512 and VAES at build time; its quality is that of a fast "
        "hash-table hash (about foldhash fast on SMHasher), not of xxh3 or rapidhash.",
    ]
    out.append('<section><h2>Notes</h2><ul class="notes">' + "".join(f"<li>{n}</li>" for n in notes) + "</ul></section>")
    return "".join(out)

def write(path, body):
    css = f"""
:root{{--bg:#f6f8fa;--card:#fff;--border:#d8dee4;--text:{COL["text"]};--muted:{COL["muted"]}}}
*{{box-sizing:border-box}}body{{margin:0;background:var(--bg);color:var(--text);font:15px/1.5 system-ui,-apple-system,"Segoe UI",sans-serif}}
main{{max-width:1300px;margin:0 auto;padding:20px 16px}}
h1{{font-size:24px;font-weight:600;margin:4px 0 14px}}
section{{background:#fbfcfd;border:1px solid var(--border);border-radius:10px;padding:20px 22px;margin-bottom:18px}}
h2{{font-size:21px;font-weight:600;margin:0 0 4px}}
.cards{{display:grid;grid-template-columns:repeat(auto-fit,minmax(200px,1fr));gap:12px}}
.card{{background:var(--card);border:1px solid var(--border);border-radius:10px;padding:14px 16px}}
.label{{font:13px {MONO};color:var(--muted);letter-spacing:.04em}}
.big{{font:600 34px {MONO};margin:6px 0 4px;letter-spacing:-.02em}}
.sub{{font:13px {MONO};color:#3d444d;font-variant-numeric:tabular-nums}}
.note{{color:#3d444d;margin:14px 0 0;max-width:1000px}}
.hint{{color:var(--muted);font-size:14px;margin-bottom:8px}}.hint b{{color:var(--text)}}
.foot{{color:var(--muted);font-size:13px;margin:8px 0 0}}
.scroll{{overflow-x:auto;background:var(--card);border:1px solid var(--border);border-radius:10px;padding:6px 10px}}
table{{border-collapse:collapse;font:13px {MONO};font-variant-numeric:tabular-nums;width:100%}}
td,th{{padding:3px 7px;text-align:right;white-space:nowrap;border-bottom:1px solid #eef0f2}}
tbody tr:last-child td{{border-bottom:0}}
th{{font:600 13px system-ui,sans-serif;color:var(--muted);vertical-align:bottom}}th.t{{color:{COL["treatment"]}}}
th.vs{{text-align:left}}td.size,th:first-child{{text-align:left;padding-left:2px}}
td.best{{font-weight:700;background:#dafbe1}}td.near{{background:#f0fbf3}}
td.iv,th.iv{{padding:0 4px;text-align:center}}td.iv svg,th.iv svg{{display:block}}
th.iv svg text{{font:11px {MONO};fill:var(--muted)}}
td.pct{{font-weight:600;padding-left:2px}}td.who{{text-align:left;color:var(--muted);padding-left:4px}}
ul.notes{{margin:0;padding-left:18px;font-size:14px;color:#3d444d}}ul.notes li{{margin-bottom:4px}}
@media (max-width:520px){{section{{padding:14px 12px}}}}
"""
    Path(path).write_text(f'<!doctype html><html lang="en"><head><meta charset="utf-8">'
                          f'<meta name="viewport" content="width=device-width,initial-scale=1">'
                          f'<title>Hash latency</title><style>{css}</style></head><body><main>{body}</main></body></html>')

def summary(rows):
    lines = [f"{'size':>8} " + " ".join(f"{LABELS[h]:>10}" for h in HASHES) + "   hothash vs fastest other"]
    for size, r in rows.items():
        p, l, h = r["change"]
        lines.append(f"{fmt_size(size):>8} " + " ".join(f"{fmt_lat(r['p50'][h_], True):>10}" for h_ in HASHES)
                     + f"   {fmt_pct(p):>5} [{fmt_pct(l)}, {fmt_pct(h)}] vs {LABELS[r['best']]}")
    return "\n".join(lines)

def selftest():
    rnd = random.Random(1)
    sizes = [1, 8, 32, 256, 4096, 1 << 20, 32 << 30]
    planted = {1: 2.0, 8: 0.92, 32: 0.6, 256: 0.75, 4096: 0.4, 1 << 20: 1.10, 32 << 30: 0.66}
    model = {h: (2 + 1.5 * k, 0.02 * (1 + (k * 7) % 5)) for k, h in enumerate(HASHES) if h != TREATMENT}
    base = {(s, h): a + b * s for s in sizes for h, (a, b) in model.items()}
    for s in sizes:
        best = min(model, key=lambda h: base[(s, h)])
        base[(s, TREATMENT)] = planted[s] * base[(s, best)]
    want_best = {s: min(model, key=lambda h: base[(s, h)]) for s in sizes}
    warmup, rounds, preempted, short = 5, 1000, 0, 0
    fd, path = tempfile.mkstemp(suffix=".jsonl")
    with os.fdopen(fd, "w") as f:
        drift = 1.0
        for r in range(rounds):
            drift *= math.exp(rnd.gauss(0, 0.003))
            for s in sizes:
                if s == sizes[-1] and r % 10:
                    continue
                n = 1 if s == sizes[-1] else 1000
                for h in HASHES:
                    ns = base[(s, h)] * drift * math.exp(rnd.gauss(0, 0.03))
                    wall = cpu = int(ns * n)
                    if rnd.random() < 0.01:
                        ns, wall = ns * 50, wall + 2 * max(PREEMPT_NS, int(wall * PREEMPT_SHARE))
                        preempted += r >= warmup
                    elif n == 1 and rnd.random() < 0.3:
                        cpu = wall - int(wall * PREEMPT_SHARE / 2)
                        short += r >= warmup
                    f.write(json.dumps({"round": r, "size": s, "hash": h, "ns": ns, "n": n,
                                        "wall_ns": wall, "cpu_ns": cpu}) + "\n")
    data, dropped = load(path, warmup)
    os.unlink(path)
    rows = analyse(data)
    ok = sum(d for d, _ in dropped.values()) == preempted
    if not ok:
        print(f"selftest: planted {preempted} preempted batches, dropped {sum(d for d, _ in dropped.values())}")
    misses = 0
    for s in sizes:
        r = rows[s]
        want = (planted[s] - 1) * 100
        p, l, h = r["change"]
        misses += not l <= want <= h
        if r["best"] != want_best[s] or abs(p - want) > 2 or not l <= want <= h:
            print(f"selftest {s} B: planted {want:+.1f}% vs {want_best[s]}, got {p:+.1f}% [{l:+.1f}, {h:+.1f}] vs {r['best']}")
        ok &= r["best"] == want_best[s] and abs(p - want) <= 2
    ok &= misses <= 1
    print(f"pipeline check: {'ok' if ok else 'FAILED'} ({len(sizes)} planted ratios from −60% to +100%, "
          f"{len(sizes) - misses} covered by their interval; {preempted} preempted batches dropped, {short} "
          f"long ones with short losses kept)")
    return ok

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("samples", nargs="?")
    ap.add_argument("--meta")
    ap.add_argument("--warmup", type=int, default=20)
    ap.add_argument("--out", default="report.html")
    ap.add_argument("--selftest", action="store_true")
    args = ap.parse_args()
    if args.selftest:
        sys.exit(0 if selftest() else 1)
    meta = json.loads(Path(args.meta).read_text()) if args.meta else {}
    data, dropped = load(args.samples, args.warmup)
    rows = analyse(data)
    write(args.out, build(rows, meta, args.warmup, dropped))
    print(summary(rows))
    print(environment(meta, dropped))
    print(f"wrote {args.out}")

if __name__ == "__main__":
    main()
