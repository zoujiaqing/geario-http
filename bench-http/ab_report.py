#!/usr/bin/env python3
"""Paired A/B for one change.

Each round measures both arms seconds apart, so the per-round ratio cancels
machine drift. A bootstrap interval on the median of those ratios says
whether the change did anything; if it contains zero, this run did not
resolve it, and the median alone would be a guess.
"""
import collections
import random
import statistics as st
import sys


def main() -> int:
    header, rows, bad = "", collections.defaultdict(dict), 0
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        if line.startswith("#"):
            if line.startswith("# ab"):
                header = line[2:]
            continue
        f = line.split()
        if len(f) != 5 or f[2] == "FAILED":
            bad += 1
            continue
        if int(f[4]):
            bad += 1
            continue
        rows[int(f[0])][f[1]] = (float(f[2]), float(f[3]))

    complete = [r for r, v in rows.items() if ("on" in v and "off" in v) or ("A" in v and "B" in v)]
    if len(complete) < 4:
        print(f"only {len(complete)} complete rounds")
        return 1

    ka, kb = ("on", "off") if "on" in rows[complete[0]] else ("A", "B")
    on = [rows[r][ka][0] for r in complete]
    off = [rows[r][kb][0] for r in complete]
    deltas = [(a - b) / b * 100 for a, b in zip(on, off)]

    rng = random.Random(4242)
    meds = sorted(st.median(rng.choices(deltas, k=len(deltas))) for _ in range(20000))
    lo, hi = meds[500], meds[-501]

    print(f"config   {header}")
    print(f"rounds   {len(complete)}" + (f", {bad} dropped" if bad else ""))
    print(f"  {ka:<12}  median {st.median(on):>9,.0f} qps")
    print(f"  {kb:<12}  median {st.median(off):>9,.0f} qps")
    print()
    print(f"  paired delta  median {st.median(deltas):+7.2f}%"
          f"   95% CI [{lo:+.2f}%, {hi:+.2f}%]")
    print(f"  spread        {min(deltas):+.2f}% .. {max(deltas):+.2f}%")
    print()
    if lo < 0 < hi:
        print("  VERDICT  the interval contains zero: this run does not separate them.")
    elif lo > 0:
        print(f"  VERDICT  {ka} ahead of {kb} by at least {lo:.2f}%.")
    else:
        print(f"  VERDICT  {ka} behind {kb} by at least {abs(hi):.2f}%.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
