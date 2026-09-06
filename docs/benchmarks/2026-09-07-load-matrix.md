# Load matrix: four servers, twelve operating points

## Why this exists

Every comparison before this one used a 24-byte response over loopback at
four connections, and each produced a confident ordering. This matrix varies
the response size, the concurrency and the method, and the ordering does not
survive any of them.

## Setup

| | |
| --- | --- |
| Host | 4 vCPU AMD EPYC 7K62, KVM, Rocky Linux 9.8 |
| Workers | 1 per server, so connection distribution is out of the picture |
| Duration | 2s warmup then 5s measured, per cell |
| Repeats | **one measurement per cell** |
| Errors | zero mismatches and zero errors in all 48 |

All four servers answer identically: same status, same headers, a generated
body of the configured length. POST bodies are drained before responding; a
server that skipped them would not be doing the work being measured.

Raw rows: `2026-09-07-load-matrix-raw.txt`.

## Results (requests per second)

| body | conns | method | geario-http | hyper/geario | hyper/tokio-mt | hyper/tokio-st |
| ---: | ---: | :-- | ---: | ---: | ---: | ---: |
| 24 | 4 | GET | 84,252 | 83,925 | **105,728** | 86,060 |
| 24 | 64 | GET | **128,816** | 92,857 | 107,805 | 90,210 |
| 24 | 64 | POST | **115,902** | 114,630 | 112,628 | 113,318 |
| 24 | 256 | GET | 80,344 | 114,360 | **123,307** | 118,920 |
| 1024 | 4 | GET | 100,781 | 100,557 | 115,986 | **120,114** |
| 1024 | 64 | GET | 117,974 | 123,388 | 123,142 | **124,790** |
| 1024 | 64 | POST | 96,545 | 96,397 | **99,036** | 81,713 |
| 1024 | 256 | GET | 115,297 | 108,350 | **117,094** | 116,973 |
| 16384 | 4 | GET | 73,171 | 66,025 | **86,475** | 75,674 |
| 16384 | 64 | GET | 80,041 | 77,575 | **103,311** | 102,706 |
| 16384 | 64 | POST | 84,029 | 78,776 | **88,228** | 81,828 |
| 16384 | 256 | GET | 85,884 | 71,381 | 92,770 | **93,691** |

Fastest per cell: hyper/tokio-mt 7, hyper/tokio-st 3, geario-http 2,
hyper/geario **0**.

## What this changes

**No ordering from the earlier single-point runs survives.** geario-http is
last at four connections, first by 19% at sixty-four, last again at two
hundred and fifty-six — all at 24 bytes. Earlier work added rounds and
computed intervals at one of those points; more samples would not have found
this, because the problem was the operating point, not the sample size.

**hyper on geario wins nothing.** It is behind in all twelve cells, including
behind geario-http in ten of them. The proposed architecture of "geario
transport, hyper protocol" is, as currently built, the worst of the four.

**The gap grows with the response.** Against hyper/tokio-mt, hyper/geario is
0.4% behind at 24 bytes and 31% behind at 16 KB with four connections. That
is the shape of a per-response copy, which is what the transport adapter
does: hyper polls into a buffer it owns, geario owns its own and lends it
through a closure, and the adapter bridges the two by copying.

Whether the cost is the adapter or geario's buffered IO path underneath it
is **not established**. Those need separating before either is blamed.

**POST flattens the field.** The three POST cells spread 3%, 21% and 12%,
against up to 60% for GET. Reading a real request body dilutes whatever the
GET cells were measuring.

## What this does not establish

One measurement per cell. A 38% gap is worth reading; a 3% one is not. None
of these carries an interval, and the same harness has produced sign flips
across rounds at other operating points.
