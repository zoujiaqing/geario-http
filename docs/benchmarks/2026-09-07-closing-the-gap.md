# Closing the gap against hyper on tokio

- Date: 2026-09-07
- Host: the Linux test server, KVM guest, RHEL 9 kernel
- Method: `ab2.sh`, twelve paired rounds, the two arms measured seconds apart
  inside each round and compared per round. Reported as the mean of the
  per-round deltas with a bootstrap 95% CI over 20,000 resamples.
- Arms: `server-hyper-on-geario` against `server-hyper-st`. Both run upstream
  hyper; only the IO layer differs. tokio is single threaded to match geario's
  one worker.

## Where it started

hyper on geario was **-31.84%, 95% CI [-35.51%, -26.81%]** against hyper on
tokio at 16 KB responses. Syscalls per request explained the shape of it:

| | geario | tokio |
| --- | --- | --- |
| write | 2.06 | 0 |
| writev | 0 | 1.00 |
| read / recvfrom | 1.06 | 1.00 |
| epoll_ctl | 1.13 | 0.00 |
| **total** | **4.40** | **2.07** |

## Three findings

**The adapter capped every write at the write watermark.** A 16 KB response
plus headers straddles it, so it took two accept-and-write cycles, and geario
only reaches for `writev` when more than one page is queued. Removing the cap
took `write` from 2.06 to 0.06 and produced one `writev` per request. Total
4.40 to 3.40.

**Streams were registered one-shot.** Every readiness the poller delivered
disarmed the fd, so every wakeup paid an `epoll_ctl` to re-arm, and a
request/response workload wakes up once per request. Level-triggered
registration keeps the interest across delivery, so the re-arm is only needed
when the interest actually changes. 1.13 to 0.13. Total 3.40 to 2.40, against
tokio's 2.07.

At that point the gap was **-10.40%, 95% CI [-15.21%, -5.11%]**.

**The remaining gap was one copy, and it was visible in the shape of the
data.** At 1 KB responses geario was already at parity, **+3.19%, 95% CI
[-4.92%, +11.12%]**; at 16 KB it was -10.40%. A deficit proportional to the
bytes written is a copy, and `__memmove_avx_unaligned_erms` was 3.71% of
geario's profile and absent from tokio's. A response written through the write
buffer is copied into the buffer and again into the socket; tokio's transport
hands hyper's slices straight to `writev`.

`IoRef::try_write_vectored` does the same, declining when bytes are queued
ahead, when the filter chain transforms the write buffer, or when the driver
has no support for it.

## Where it ended

| Case | geario against hyper on tokio |
| --- | --- |
| HTTP/1.1, 1 KB, 64 conns | +3.19%, 95% CI [-4.92%, +11.12%] |
| HTTP/1.1, 16 KB, 64 conns | **-2.15%, 95% CI [-6.80%, +3.05%]** |
| HTTP/2, 16 KB, 4 conns x 16 streams | **-1.07%, 95% CI [-7.15%, +5.41%]** |
| HTTP/1.1, 16 KB, 64 conns, io_uring driver | -6.51%, 95% CI [-12.11%, -0.83%] |

Every interval except io_uring's contains zero: at these sizes and connection
counts the two are not distinguishable. The user-space memmove is gone and the
kernel copy matches tokio's, 13.09% against 14.07%.

## The io_uring driver

It produced 0 QPS and wrong responses before this work. It now serves
correctly, 0 mismatches across twelve rounds at 64 connections, because the
direct write path bypasses the ring's send path, which is where the fault
was. **The fault has not been found, only avoided**: the ring's send path is
still used whenever the socket is full and the write has to be buffered.

At 64 connections on this host io_uring is slower than the polling driver, so
polling remains the better default here. That is one KVM guest; it is not a
statement about io_uring.

## What is not claimed

These are single-worker measurements at 16 KB and 1 KB on one host. They say
geario's IO layer is not costing anything against tokio at these shapes. They
do not say it is faster, and nothing here was measured above 64 connections or
across many workers.
