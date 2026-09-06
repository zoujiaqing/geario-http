#!/bin/bash
# A/B one change, alternating inside each round.
#
# The load matrix cannot evaluate a change: between two runs of it, servers
# whose code never changed moved by -30% to +48%. Anything smaller than that
# drowns. Pairing the two arms within a round cancels the drift, which is the
# only way a change of a few percent becomes visible.
set -u

ROUNDS="${ROUNDS:-12}"
SECS="${SECS:-6}"
CONNS="${CONNS:-64}"
BODY="${BODY:-16384}"
POST="${POST:-0}"
BIN_DIR="$(cd "$(dirname "$0")" && pwd)"
[ -x "$BIN_DIR/target/release/client" ] && BIN_DIR="$BIN_DIR/target/release"
SERVER="${SERVER:-server-hyper-on-geario}"
PORT="${PORT:-18093}"

PID=""
cleanup() { [ -n "$PID" ] && { kill -9 "$PID" 2>/dev/null; wait "$PID" 2>/dev/null; }; }
trap cleanup EXIT INT TERM

measure() {  # $1 = value for GEARIO_NO_VECTORED
    GEARIO_NO_VECTORED="$1" BENCH_ADDR="127.0.0.1:$PORT" BENCH_WORKERS=1 \
        BENCH_BODY_SIZE="$BODY" "$BIN_DIR/$SERVER" >/dev/null 2>&1 &
    PID=$!
    local up=0 i
    for ((i=0; i<100; i++)); do
        nc -z 127.0.0.1 "$PORT" 2>/dev/null && { up=1; break; }
        sleep 0.1
    done
    if [ "$up" -eq 0 ]; then echo "FAILED 0 1"; kill -9 "$PID" 2>/dev/null; PID=""; return; fi
    BENCH_BODY_SIZE="$BODY" "$BIN_DIR/client" "127.0.0.1:$PORT" "$CONNS" 2 "$POST" >/dev/null 2>&1
    BENCH_BODY_SIZE="$BODY" "$BIN_DIR/client" "127.0.0.1:$PORT" "$CONNS" "$SECS" "$POST" \
        | awk '/^qps/{q=$2} /^p50/{p=$2} /^mismatches/{m=$2} /^errors/{e=$2} END {print q, p, m+e}'
    kill "$PID" 2>/dev/null; wait "$PID" 2>/dev/null; PID=""
}

echo "# ab body=$BODY conns=$CONNS post=$POST rounds=$ROUNDS secs=$SECS"
echo "# round arm qps p50 bad"
for ((r=1; r<=ROUNDS; r++)); do
    # Alternate which arm goes first, so drift inside a round does not always
    # land on the same one.
    if (( r % 2 )); then arms="on off"; else arms="off on"; fi
    for arm in $arms; do
        [ "$arm" = "on" ] && v=0 || v=1
        echo "$r $arm $(measure "$v")"
    done
done
