#!/usr/bin/env python3
"""Statistics for the geario/hyper comparison.

Reads the rows run.sh writes:

    round server qps p50_us p99_us bad cpu_ms

Two things this is careful about, both from getting them wrong first.

The headline is the *paired* difference: each round measures every server
within seconds of the others, so a per-round ratio cancels whatever the
machine was doing at the time. That is a different number from the ratio of
the medians, and both are printed with their names on them rather than one
being passed off as the other.

And the spread is reported as a bootstrap interval rather than as the
smallest sample. "Every round agreed, so it is at least X%" treats a minimum
as a bound, which it is not.
"""
import collections
import random
import statistics as st
import sys


def median(xs):
    return st.median(xs)


def bootstrap_ci(xs, iters=20000, alpha=0.05, seed=12345):
    """Percentile interval for the median. Small samples, no distributional
    assumption available, so resample instead of assuming one."""
    if len(xs) < 2:
        return (float("nan"), float("nan"))
    rng = random.Random(seed)
    meds = []
    n = len(xs)
    for _ in range(iters):
        meds.append(st.median(rng.choices(xs, k=n)))
    meds.sort()
    lo = meds[int(iters * alpha / 2)]
    hi = meds[int(iters * (1 - alpha / 2)) - 1]
    return (lo, hi)


def main() -> int:
    header = ""
    SECS = None
    rows = collections.defaultdict(dict)
    failures = []
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        if line.startswith("#"):
            if "conns=" in line:
                header = line.lstrip("# ")
                for tok in header.split():
                    if tok.startswith("secs="):
                        SECS = float(tok.split("=")[1])
            continue
        f = line.split()
        if len(f) != 7:
            failures.append(f"malformed row: {line}")
            continue
        rnd, server = f[0], f[1]
        if f[2] == "FAILED_TO_BIND":
            failures.append(f"round {rnd}: {server} never bound")
            continue
        try:
            qps, p50, p99, bad, cpu = (
                float(f[2]), float(f[3]), float(f[4]), int(f[5]), float(f[6]),
            )
        except ValueError:
            failures.append(f"unparsable row: {line}")
            continue
        if bad:
            failures.append(f"round {rnd}: {server} had {bad} bad response(s)")
            continue
        rows[int(rnd)][server] = (qps, p50, p99, cpu)

    if SECS is None:
        print("header did not carry secs=; cannot compute CPU per request")
        return 1

    servers = sorted({s for r in rows.values() for s in r})
    if not servers:
        print("no usable rows")
        return 1

    # Only rounds where every server reported can be paired.
    complete = [r for r, v in rows.items() if len(v) == len(servers)]
    dropped = len(rows) - len(complete)

    print(f"config    {header}")
    print(f"rounds    {len(complete)} complete"
          + (f", {dropped} incomplete" if dropped else ""))
    if failures:
        print(f"problems  {len(failures)}")
        for f in failures[:8]:
            print(f"          {f}")

    print("\nper-server medians")
    print(f"  {'server':<22} {'qps':>9} {'p50':>9} {'p99':>9} {'cpu':>13}")
    med_qps = {}
    for s in servers:
        q = [rows[r][s][0] for r in complete]
        p = [rows[r][s][1] for r in complete]
        n = [rows[r][s][2] for r in complete]
        c = [rows[r][s][3] for r in complete]
        med_qps[s] = median(q)
        # CPU milliseconds per thousand requests. The first version of this
        # divided by qps and forgot the duration, inflating every figure by
        # the length of a round.
        per = [
            ci / (qi * SECS) * 1000 if qi else 0
            for ci, qi in zip(c, [rows[r][s][0] for r in complete])
        ]
        print(f"  {s:<22} {median(q):>9,.0f} {median(p):>8.1f}us {median(n):>8.1f}us"
              f" {median(per):>9.2f}ms/kreq")

    base = servers[0]
    print(f"\npaired differences against {base}")
    print("  (median of per-round ratios, with a bootstrap 95% interval;")
    print("   the ratio of the medians is shown separately because it is a")
    print("   different statistic and the two do not have to agree)")
    for s in servers[1:]:
        d = [
            (rows[r][base][0] - rows[r][s][0]) / rows[r][s][0] * 100
            for r in complete
        ]
        lo, hi = bootstrap_ci(d)
        ratio_of_medians = (med_qps[base] - med_qps[s]) / med_qps[s] * 100
        agree = "same sign in every round" if (min(d) > 0 or max(d) < 0) else "sign changes"
        print(f"\n  vs {s}")
        print(f"    paired median      {median(d):+7.2f}%   95% CI [{lo:+.2f}%, {hi:+.2f}%]")
        print(f"    ratio of medians   {ratio_of_medians:+7.2f}%")
        print(f"    observed spread    {min(d):+.2f}% .. {max(d):+.2f}%   ({agree})")
        if lo < 0 < hi:
            print("    → the interval contains zero; this run does not separate them.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
