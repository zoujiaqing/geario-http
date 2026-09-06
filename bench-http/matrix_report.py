#!/usr/bin/env python3
"""Summarise the load matrix.

Rows are `body conns method server qps p50 p99 bad`.

One measurement per cell, so nothing here carries an interval. The point of
the matrix is the *shape*: whether the ordering between servers holds as the
response grows and concurrency rises, or whether it depends on the load. A
single cell is an observation; a consistent ordering across cells is a
finding, and an inconsistent one is a warning against quoting any of them.
"""
import collections
import sys

SHORT = {
    "server-geario": "geario-http",
    "server-hyper-on-geario": "hyper/geario",
    "server-hyper": "hyper/tokio-mt",
    "server-hyper-st": "hyper/tokio-st",
}
ORDER = ["server-geario", "server-hyper-on-geario", "server-hyper", "server-hyper-st"]


def main() -> int:
    cells = collections.defaultdict(dict)
    bad = []
    for line in sys.stdin:
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        f = line.split()
        if len(f) != 8:
            continue
        body, conns, method, server = f[0], f[1], f[2], f[3]
        if f[4] == "FAILED_TO_BIND":
            bad.append(f"{body}/{conns}/{method} {server}: never bound")
            continue
        qps, p50, p99, errs = float(f[4]), float(f[5]), float(f[6]), int(f[7])
        if errs:
            bad.append(f"{body}/{conns}/{method} {server}: {errs} bad")
            continue
        cells[(int(body), int(conns), method)][server] = (qps, p50, p99)

    if bad:
        print(f"problems ({len(bad)}):")
        for b in bad[:10]:
            print(f"  {b}")
        print()

    print(f"{'body':>6} {'conns':>6} {'meth':>5} "
          + " ".join(f"{SHORT[s]:>15}" for s in ORDER))
    print(f"{'':>6} {'':>6} {'':>5} " + " ".join(f"{'qps':>15}" for _ in ORDER))
    wins = collections.Counter()
    for key in sorted(cells):
        body, conns, method = key
        row = cells[key]
        vals = [row.get(s, (0,))[0] for s in ORDER]
        best = max(vals) if vals else 0
        cells_txt = []
        for s, v in zip(ORDER, vals):
            mark = "*" if v and v == best else " "
            cells_txt.append(f"{v:>14,.0f}{mark}")
        print(f"{body:>6} {conns:>6} {method:>5} " + " ".join(cells_txt))
        for s, v in zip(ORDER, vals):
            if v and v == best:
                wins[s] += 1

    print("\nfastest in each cell")
    for s in ORDER:
        print(f"  {SHORT[s]:<16} {wins[s]:>2} / {len(cells)}")

    # Does the ordering hold, or does it depend on the load?
    print("\ngeario-http against hyper/geario, per cell")
    flips = 0
    for key in sorted(cells):
        row = cells[key]
        a = row.get("server-geario", (0,))[0]
        b = row.get("server-hyper-on-geario", (0,))[0]
        if a and b:
            d = (a - b) / b * 100
            if d > 0:
                flips += 1
            print(f"  {key[0]:>6}B {key[1]:>4}c {key[2]:<5} {d:+7.1f}%")
    print(f"\n  geario-http ahead in {flips} of {len(cells)} cells")
    return 0


if __name__ == "__main__":
    sys.exit(main())
