# The native HTTP/1 stack against hyper, on geario's IO

- Date: 2026-09-07
- Question: geario-http carries its own HTTP/1 stack, 4,858 lines, which the
  FFI uses, and a 489-line adapter that runs hyper, which the benchmarks use.
  Is the native stack worth its size?
- Method: `ab2.sh`, twelve paired rounds at each host's knee, bootstrap 95% CI.
  Both servers answer with the same status, the same content-length and the
  same content-type, and the generator checks the response rather than
  counting it.

## Numbers

On Fedora 44, the host whose intervals are tight enough to resolve a few
points:

| | against hyper on tokio | against hyper on geario |
| --- | --- | --- |
| native, 16 KB, 4 conns | **+4.45%** [+3.69%, +5.19%] | +2.31% [+1.18%, +3.56%] |
| hyper on geario, 16 KB | +1.20% [+0.14%, +2.33%] | — |
| native, 1 KB, 8 conns | **-3.84%** [-5.53%, -2.10%] | +3.53% [+1.70%, +5.86%] |
| hyper on geario, 1 KB | -5.04% [-7.20%, -2.85%] | — |

On Rocky 9, 16 KB at eight connections, native against hyper on geario:
-4.21% [-8.78%, +0.56%]. That interval contains zero and contradicts the
Fedora sign; Rocky could not resolve four points in any earlier comparison
either, and nothing is concluded from it.

The two ways of reaching the same comparison agree. At 16 KB, native is
+4.45% over tokio and hyper-on-geario is +1.20%, which puts native about
three points over hyper-on-geario; measured directly it is +2.31%. At 1 KB
the same arithmetic gives about one point against a measured +3.53%. Both
are within what separate runs drift by.

## What that buys

The native stack is worth roughly two to four points over the hyper adapter,
consistently, on the host that can measure it. For 4,858 lines against 489.

## What it costs, which is not lines

It has no HTTP/2 and no path to one. That is not an abstract gap:

  hyper4k's client reports HTTP/2 in its capability bits and has a test
  asserting the bit is set. geario's FFI reports HTTP/1 only, because it is
  built on the native stack.

So the FFI cannot replace hyper4k today without taking HTTP/2 away from
whatever uses it. The hyper adapter already has HTTP/2 on both ends, tested
over a real socket with sixteen concurrent streams.

The native stack also carries an HTTP/1 parser and dispatcher, 64 inline
tests, in a class of code where the interesting failures are request
smuggling and desynchronisation rather than crashes.

## What this does not settle

Deleting the native stack is not a matter of deleting `src/h1`. geario-http's
own client, its WebSocket support, and the compression and cookie layers are
all built on its codec. Pointing the FFI at hyper is a small change; removing
the native stack is a large one, and these numbers do not decide it.
