#!/bin/bash
# Paired A/B between two binaries, alternating inside each round.
#
# Same discipline as ab.sh: the matrix drifts by tens of percent between runs,
# so the two arms have to be measured seconds apart and compared per round.
set -u
ROUNDS="${ROUNDS:-12}"; SECS="${SECS:-5}"; CONNS="${CONNS:-64}"
BODY="${BODY:-16384}"; POST="${POST:-0}"; WORKERS="${WORKERS:-1}"
A="${A:-srv-uring}"; B="${B:-srv-polling}"
# The h2 generator takes a stream count as a fourth argument; POST is h1 only.
CLIENT="${CLIENT:-client}"; STREAMS="${STREAMS:-16}"
PORT_A="${PORT_A:-18093}"; PORT_B="${PORT_B:-18094}"
D="$(cd "$(dirname "$0")" && pwd)"

PID=""
cleanup() { [ -n "$PID" ] && { kill -9 "$PID" 2>/dev/null; wait "$PID" 2>/dev/null; }; }
trap cleanup EXIT INT TERM

measure() {  # bin port
    BENCH_ADDR="127.0.0.1:$2" BENCH_WORKERS="$WORKERS" BENCH_BODY_SIZE="$BODY" \
        "$D/$1" >/dev/null 2>&1 &
    PID=$!
    local up=0 i
    for ((i=0;i<100;i++)); do nc -z 127.0.0.1 "$2" 2>/dev/null && { up=1; break; }; sleep 0.1; done
    if [ "$up" -eq 0 ]; then echo "FAILED 0 1"; kill -9 "$PID" 2>/dev/null; PID=""; return; fi
    [ "$CLIENT" = client ] && arg4="$POST" || arg4="$STREAMS"
    BENCH_BODY_SIZE="$BODY" "$D/$CLIENT" "127.0.0.1:$2" "$CONNS" 2 "$arg4" >/dev/null 2>&1
    BENCH_BODY_SIZE="$BODY" "$D/$CLIENT" "127.0.0.1:$2" "$CONNS" "$SECS" "$arg4" \
        | awk '/^qps/{q=$2}/^p50/{p=$2}/^mismatches/{m=$2}/^errors/{e=$2}END{print q,p,m+e}'
    kill "$PID" 2>/dev/null; wait "$PID" 2>/dev/null; PID=""
}

echo "# ab $A vs $B client=$CLIENT body=$BODY conns=$CONNS post=$POST streams=$STREAMS workers=$WORKERS rounds=$ROUNDS secs=$SECS"
echo "# round arm qps p50 bad"
for ((r=1;r<=ROUNDS;r++)); do
    if (( r % 2 )); then first=A; else first=B; fi
    for who in $first $([ "$first" = A ] && echo B || echo A); do
        [ "$who" = A ] && { bin=$A; port=$PORT_A; } || { bin=$B; port=$PORT_B; }
        echo "$r $who $(measure "$bin" "$port")"
    done
done
