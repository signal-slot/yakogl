#!/usr/bin/env python3
"""Decode the PowerVR HWPerf drain console lines into per-kick timing tables.

Usage: decode.py [--core-hz HZ] [FILE]      (stdin when FILE is absent or -)
See README.md for the line format and its source.
"""
import argparse
import re
import sys

KV = re.compile(r"(\w+)=(\S+)")
LANE_ORDER = ["ta", "3d", "3dspm", "3dtq", "cdm", "tla", "tdm"]


def kv(s):
    return {k: v for k, v in KV.findall(s)}


def us(ticks, hz):
    return ticks * 1e6 / hz


def fmt_us(x):
    return "-" if x is None else "%.2f" % x


def parse(lines):
    drains = []
    for ln in lines:
        m = re.search(r"pvr-hwperf(:| lane=| job )", ln)
        if not m:
            continue
        if m.group(1) == ":":
            d = {"head": kv(ln[m.end():]), "lanes": {}, "jobs": []}
            drains.append(d)
        elif drains:
            f = kv(ln[m.start():])
            if m.group(1) == " job ":
                drains[-1]["jobs"].append(f)
            else:
                drains[-1]["lanes"][f["lane"]] = f
    return drains


def table(hdr, rows):
    w = [max(len(str(r[i])) for r in [hdr] + rows) for i in range(len(hdr))]
    out = []
    for r in [hdr] + rows:
        out.append("  ".join(str(c).rjust(w[i]) if i else str(c).ljust(w[i]) for i, c in enumerate(r)).rstrip())
    return out


def render(drains, hz):
    out = []
    for n, d in enumerate(drains, 1):
        h = d["head"]
        out.append("drain %d: from=%s widx=%s packets=%s tasks=%s dropped=%s fw_drops=%s span_us=%s" % (
            n, h.get("from", "?"), h.get("widx", "?"), h.get("packets", "?"), h.get("tasks", "?"),
            h.get("dropped", "?"), h.get("fw_drops", "?"),
            fmt_us(us(int(h["span_ticks"]), hz)) if "span_ticks" in h else "?"))
        jobs = {}
        for j in d["jobs"]:
            jobs.setdefault(j.get("lane", "?"), []).append(j)
        hdr = ["lane", "count", "total_us", "mean_us", "max_us", "k2s_n", "k2s_mean_us"]
        rows = []
        names = sorted(d["lanes"], key=lambda x: LANE_ORDER.index(x) if x in LANE_ORDER else 99)
        for name in names:
            f = d["lanes"][name]
            cnt = int(f.get("tasks", 0))
            tot = int(f.get("gpu_ticks", 0))
            kn = int(f.get("k2s_n", 0))
            kt = int(f.get("k2s_ticks", 0))
            mx = None
            if name in jobs:
                mx = max(us(int(j.get("gpu_ticks", 0)), hz) for j in jobs[name])
            rows.append([name, cnt, fmt_us(us(tot, hz)), fmt_us(us(tot, hz) / cnt) if cnt else "-",
                         fmt_us(mx), kn, fmt_us(us(kt, hz) / kn) if kn else "-"])
        out += table(hdr, rows)
        if d["jobs"]:
            agg = {}
            for j in d["jobs"]:
                a = agg.setdefault((j.get("ext", "?"), j.get("lane", "?")), [0, 0, 0, 0])
                a[0] += 1
                a[1] += int(j.get("gpu_ticks", 0))
                a[2] += int(j.get("k2s_ticks", 0))
                a[3] = max(a[3], int(j.get("gpu_ticks", 0)))
            rows = [[e, l, a[0], fmt_us(us(a[1], hz)), fmt_us(us(a[3], hz)), fmt_us(us(a[2], hz) / a[0])]
                    for (e, l), a in sorted(agg.items(), key=lambda kv_: (str(kv_[0][0]).zfill(12), kv_[0][1]))]
            out.append("per job ref:")
            out += table(["ext_job_ref", "lane", "count", "total_us", "max_us", "k2s_mean_us"], rows)
        out.append("")
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--core-hz", type=int, default=800000000, help="GPU core clock (default 800000000)")
    ap.add_argument("file", nargs="?", default="-")
    a = ap.parse_args()
    f = sys.stdin if a.file == "-" else open(a.file, errors="replace")
    drains = parse(f)
    if not drains:
        print("no pvr-hwperf lines found", file=sys.stderr)
        return 1
    print("\n".join(render(drains, a.core_hz)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
