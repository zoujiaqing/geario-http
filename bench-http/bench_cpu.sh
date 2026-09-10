#!/bin/bash
# Paired A/B with CPU-per-request and RSS, alternating arms within each round.
#
# The server is started and stopped in THIS shell (measure is called directly,
# not in a command substitution), so the PID it sets is visible to the EXIT/INT
# trap and a Ctrl-C mid-run kills the server instead of orphaning it. An earlier
# version ran measure in $(...), where the trap could never see the PID.
#
# Any collection failure -- server did not come up, warmup or timed client
# exited non-zero, or a metric is missing -- makes the round INVALID
# (bad field = 1), rather than being silently counted as zero.
#
# Env: A B BODY CONNS ROUNDS SECS WORKERS PORT_A PORT_B CLIENT
set -u -o pipefail
ROUNDS="${ROUNDS:-16}"; SECS="${SECS:-5}"; CONNS="${CONNS:-64}"
BODY="${BODY:-16384}"; WORKERS="${WORKERS:-1}"
A="${A:-geario-before}"; B="${B:-geario-after}"
CLIENT="${CLIENT:-client}"
PORT_A="${PORT_A:-18093}"; PORT_B="${PORT_B:-18094}"
D="$(cd "$(dirname "$0")" && pwd)"
PID=""
cleanup(){ [ -n "$PID" ] && { kill -9 "$PID" 2>/dev/null; wait "$PID" 2>/dev/null; }; PID=""; }
trap cleanup EXIT INT TERM
HZ=$(getconf CLK_TCK)

cpu_ticks(){ awk '{print $14+$15}' "/proc/$1/stat"; }        # errors -> empty
rss_kb(){ awk '/VmRSS/{print $2}' "/proc/$1/status"; }

RESULT=""
measure(){  # bin port ; sets RESULT="qps p50 p99 cpu_us_per_req rss_mb bad"
    local bin="$1" port="$2"
    BENCH_ADDR="127.0.0.1:$port" BENCH_WORKERS="$WORKERS" BENCH_BODY_SIZE="$BODY" "$D/$bin" >/dev/null 2>&1 &
    PID=$!; sleep 0.6
    if ! kill -0 "$PID" 2>/dev/null; then PID=""; RESULT="0 0 0 0 0 1"; return; fi
    # warmup; a failed warmup means the server is not really serving -> invalid
    if ! BENCH_BODY_SIZE="$BODY" "$D/$CLIENT" "127.0.0.1:$port" "$CONNS" 2 0 >/dev/null 2>&1; then
        cleanup; RESULT="0 0 0 0 0 1"; return
    fi
    local c0 out rc c1 rss
    c0="$(cpu_ticks "$PID")"
    out="$(BENCH_BODY_SIZE="$BODY" "$D/$CLIENT" "127.0.0.1:$port" "$CONNS" "$SECS" 0 2>/dev/null)"; rc=$?
    c1="$(cpu_ticks "$PID")"; rss="$(rss_kb "$PID")"
    cleanup
    if [ "$rc" -ne 0 ] || [ -z "$c0" ] || [ -z "$c1" ] || [ -z "$rss" ]; then
        RESULT="0 0 0 0 0 1"; return
    fi
    RESULT="$(printf '%s\n' "$out" | awk -v c0="$c0" -v c1="$c1" -v hz="$HZ" -v rss="$rss" '
        /^qps/{q=$2}/^p50/{p50=$2}/^p99/{p99=$2}/^requests/{req=$2}/^mismatches/{m=$2}/^errors/{e=$2}
        END{ if(q==""||p99==""||req==""||req==0||m==""||e==""){ print "0 0 0 0 0 1" }
             else { printf "%.0f %.1f %.1f %.2f %.1f %d\n", q, p50, p99, (c1-c0)/hz*1e6/req, rss/1024, (m+e) } }')"
    [ -z "$RESULT" ] && RESULT="0 0 0 0 0 1"
}

echo "# bench_cpu A=$A B=$B body=$BODY conns=$CONNS workers=$WORKERS rounds=$ROUNDS secs=$SECS"
echo "# round arm qps p50 p99 cpu_us_req rss_mb bad"
for ((r=1;r<=ROUNDS;r++)); do
    (( r % 2 )) && first=A || first=B
    for who in $first $([ "$first" = A ] && echo B || echo A); do
        [ "$who" = A ] && { bin=$A; port=$PORT_A; } || { bin=$B; port=$PORT_B; }
        measure "$bin" "$port"
        echo "$r $who $RESULT"
    done
done
