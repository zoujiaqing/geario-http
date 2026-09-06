# Measuring at the knee, and a second host

- Date: 2026-09-07
- Hosts:
  - **Rocky 9**, kernel 5.14, 4 cores, the original test server
  - **Fedora 44**, kernel 7.0.12, 2 cores, io_uring enabled by default
- Method: `ab2.sh`, twelve paired rounds, bootstrap 95% CI over the
  per-round deltas. Load generator and server share the host; the two are
  10 ms apart on the network, so 16 KB at 100k qps is 1.6 GB/s and running
  them on separate hosts is not possible.

## The earlier numbers were measured in the wrong place

Every measurement before this was taken at 64 connections. A connection
sweep says that is far past saturation on both hosts:

| conns | Rocky 9, 16 KB | Fedora 44, 16 KB | Fedora 44, 1 KB |
| --- | --- | --- | --- |
| 1 | 21,460 | 19,748 | 23,423 |
| 2 | 58,036 | 38,198 | 54,118 |
| 4 | 83,773 | **50,485** | 69,779 |
| 8 | **88,723** | 42,989 | **72,051** |
| 16 | 79,992 | 38,494 | 70,347 |
| 64 | 83,224 | 32,920 | 57,548 |

Past the knee the generator and the server fight for the same cores and
what gets measured is scheduling jitter. That is where the ±16 point
confidence intervals in the previous record came from. At the knee the
same comparison resolves to two points:

| | at 64 conns | at the knee |
| --- | --- | --- |
| Rocky 9, 16 KB | -4.66% [-9.61%, +0.42%] | -4.64% [-8.54%, -0.28%] |
| Fedora 44, 1 KB | — | -11.19% [-12.67%, -9.70%] |

## What that exposed

A gap at 1 KB with a tight interval is per-request overhead, not
per-byte, and syscall counts said the same thing: 2.77 per request
against tokio's 2.13. An strace of the loop showed the connections were
not responsible -- their sockets never appear in `epoll_ctl` -- and that
every turn spent five syscalls on the poller's own notifier and timerfd.

That is fixed in `geario-polling`; see the commit for what changed and
why the poller had to be forked rather than patched.

## Where it stands

| host | body | conns | geario against hyper on tokio |
| --- | --- | --- | --- |
| Fedora 44 | 16 KB | 4 | **+1.20%** [+0.14%, +2.33%] |
| Fedora 44, io_uring | 16 KB | 4 | **+1.90%** [+0.43%, +3.91%] |
| Fedora 44 | 1 KB | 8 | **-5.04%** [-7.20%, -2.85%] |
| Rocky 9 | 16 KB | 8 | -7.97% [-12.28%, -3.67%] |

Fedora, the newer kernel, has geario slightly ahead at 16 KB on both
drivers and behind at 1 KB. Rocky is the noisier host and its intervals
overlap everything.

## What is not claimed

The 16 KB numbers on Rocky moved from -4.64% to -7.97% across the poller
change, with intervals that overlap. That is not evidence the change hurt
16 KB; it is evidence Rocky cannot resolve four points. The 1 KB result on
Fedora is the one with non-overlapping intervals before and after, and it
is the one backed by exact syscall counts.

Both hosts are two- and four-core KVM guests with the generator on the
same machine. Nothing here says anything about many-core behaviour.
