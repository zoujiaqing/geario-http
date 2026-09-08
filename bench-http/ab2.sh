#!/bin/bash
# Paired A/B between two binaries, alternating inside each round.
#
# The matrix drifts by tens of percent between runs, so the two arms have to
# be measured seconds apart and compared per round.
#
# The server is started and stopped in this shell, not inside a command
# substitution, so the EXIT/INT trap can actually kill it: an earlier version
# set the PID inside `$(...)` where the trap could never see it, and a
# Ctrl-C mid-run left the server orphaned holding its port.
set -u
set -o pipefail
ROUNDS="${ROUNDS:-12}"; SECS="${SECS:-5}"; CONNS="${CONNS:-64}"
BODY="${BODY:-16384}"; POST="${POST:-0}"; WORKERS="${WORKERS:-1}"
A="${A:-srv-uring}"; B="${B:-srv-polling}"
# The h2 generator takes a stream count as a fourth argument; POST is h1 only.
CLIENT="${CLIENT:-client}"; STREAMS="${STREAMS:-16}"
PORT_A="${PORT_A:-18093}"; PORT_B="${PORT_B:-18094}"
D="$(cd "$(dirname "$0")" && pwd)"

PID=""
cleanup() { [ -n "$PID" ] && { kill -9 "$PID" 2>/dev/null; wait "$PID" 2>/dev/null; }; PID=""; }
trap cleanup EXIT INT TERM

start_server() {  # bin port
    BENCH_ADDR="127.0.0.1:$2" BENCH_WORKERS="$WORKERS" BENCH_BODY_SIZE="$BODY" \
        "$D/$1" >/dev/null 2>&1 &
    PID=$!
    local i
    for ((i=0;i<100;i++)); do nc -z 127.0.0.1 "$2" 2>/dev/null && return 0; sleep 0.1; done
    return 1
}

stop_server() { kill "$PID" 2>/dev/null; wait "$PID" 2>/dev/null; PID=""; }

# Runs the client and prints "qps p50 bad". A client that fails, or omits any
# field, prints a bad round (bad>=1) rather than a zero that reads as a real
# measurement.
run_client() {  # port
    local arg4
    [ "$CLIENT" = client ] && arg4="$POST" || arg4="$STREAMS"
    BENCH_BODY_SIZE="$BODY" "$D/$CLIENT" "127.0.0.1:$1" "$CONNS" 2 "$arg4" >/dev/null 2>&1
    BENCH_BODY_SIZE="$BODY" "$D/$CLIENT" "127.0.0.1:$1" "$CONNS" "$SECS" "$arg4" \
        | awk '/^qps/{q=$2}/^p50/{p=$2}/^mismatches/{m=$2}/^errors/{e=$2}
               END{ if (q=="" || p=="") print "0 0 1"; else print q, p, (m+e) }'
}

# Sets RESULT to "qps p50 bad". The server is started here, in this shell, so
# the trap's PID is valid; only the client runs in a command substitution,
# and it spawns nothing that outlives it.
RESULT=""
measure() {  # bin port
    if ! start_server "$1" "$2"; then stop_server; RESULT="0 0 1"; return; fi
    RESULT="$(run_client "$2")" || RESULT="0 0 1"
    stop_server
}

echo "# ab $A vs $B client=$CLIENT body=$BODY conns=$CONNS post=$POST streams=$STREAMS workers=$WORKERS rounds=$ROUNDS secs=$SECS"
echo "# round arm qps p50 bad"
for ((r=1;r<=ROUNDS;r++)); do
    if (( r % 2 )); then first=A; else first=B; fi
    for who in $first $([ "$first" = A ] && echo B || echo A); do
        [ "$who" = A ] && { bin=$A; port=$PORT_A; } || { bin=$B; port=$PORT_B; }
        measure "$bin" "$port"
        echo "$r $who $RESULT"
    done
done
