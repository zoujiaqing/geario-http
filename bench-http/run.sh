#!/bin/bash
# geario-http against hyper, alternating, order rotated every round.
#
# Emits one row per measurement to stdout and nothing else, so the raw record
# is the file you keep. report.py does the statistics.
#
# What this script has learned the hard way, each item from a real failure:
#
#   - Servers are started, recorded, killed and waited on by THIS shell. An
#     earlier version started them inside a command substitution; the pid list
#     never came back, `wait` reported "not a child of this shell", and `|| true`
#     hid it. The trap cleaned up nothing.
#   - A server that never binds is a failed run, not a zero. It says so.
#   - Every measured process gets its own warmup. Warming one and then killing
#     it warms nothing.
#   - Server order rotates, so drift inside a round does not always land on the
#     same one, and no server is permanently in the middle.
#   - Operating points come from a sweep, not a guess.
set -u

ROUNDS="${ROUNDS:-10}"
SECS="${SECS:-8}"
CONNS="${CONNS:-4}"
WORKERS="${WORKERS:-}"
SERVERS="${SERVERS:-server-geario:18090 server-hyper:18091}"

HERE="$(cd "$(dirname "$0")" && pwd)"
if [ -x "$HERE/target/release/client" ]; then BIN="$HERE/target/release"; else BIN="$HERE"; fi

CORES=$(nproc 2>/dev/null || sysctl -n hw.ncpu)
load=$(uptime | sed 's/.*averages*:[ ]*//' | awk '{print $1}' | tr -d ',')
if [ "$(awk -v l="$load" -v c="$CORES" 'BEGIN{print (l > c/2) ? 1 : 0}')" = "1" ] \
   && [ "${IGNORE_LOAD:-0}" != "1" ]; then
    echo "refusing: load average is $load on $CORES cores." >&2
    exit 1
fi

SERVER_PID=""
cleanup() {
    if [ -n "$SERVER_PID" ]; then
        kill -9 "$SERVER_PID" 2>/dev/null
        wait "$SERVER_PID" 2>/dev/null
    fi
}
trap cleanup EXIT INT TERM

# Runs one measurement. Starting and reaping happen here, in the shell that
# owns the job, which is the whole point.
measure() {  # bin port conns -> "qps p50 p99 bad cpu_ms"
    local bin=$1 port=$2 conns=$3

    if [ -n "$WORKERS" ]; then
        BENCH_ADDR="127.0.0.1:$port" BENCH_WORKERS="$WORKERS" "$BIN/$bin" >/dev/null 2>&1 &
    else
        BENCH_ADDR="127.0.0.1:$port" "$BIN/$bin" >/dev/null 2>&1 &
    fi
    SERVER_PID=$!

    local up=0 i
    for ((i = 0; i < 100; i++)); do
        if nc -z 127.0.0.1 "$port" 2>/dev/null; then up=1; break; fi
        sleep 0.1
    done
    if [ "$up" -eq 0 ]; then
        echo "FAILED_TO_BIND 0 0 1 0"
        kill -9 "$SERVER_PID" 2>/dev/null; wait "$SERVER_PID" 2>/dev/null; SERVER_PID=""
        return
    fi

    "$BIN/client" "127.0.0.1:$port" "$conns" 2 >/dev/null 2>&1

    # Server CPU time across the measurement, so throughput can be read
    # against the work that produced it.
    local before after cpu
    before=$(cpu_ms "$SERVER_PID")
    local line
    line=$("$BIN/client" "127.0.0.1:$port" "$conns" "$SECS" \
        | awk '/^qps/{q=$2} /^p50/{p=$2} /^p99/{n=$2} /^mismatches/{m=$2} /^errors/{e=$2}
               END {print q, p, n, m+e}')
    after=$(cpu_ms "$SERVER_PID")
    cpu=$((after - before))

    kill "$SERVER_PID" 2>/dev/null
    wait "$SERVER_PID" 2>/dev/null
    SERVER_PID=""
    echo "$line $cpu"
}

# utime+stime in milliseconds for a pid and all its threads.
cpu_ms() {
    if [ -r "/proc/$1/stat" ]; then
        awk -v hz="$(getconf CLK_TCK)" '{printf "%d", ($14 + $15) * 1000 / hz}' "/proc/$1/stat"
    else
        ps -o time= -p "$1" 2>/dev/null \
            | awk -F: '{printf "%d", ($1*3600 + $2*60 + $3) * 1000}' || echo 0
    fi
}

echo "# conns=$CONNS rounds=$ROUNDS secs=$SECS cores=$CORES workers=${WORKERS:-default}"
echo "# round server qps p50_us p99_us bad cpu_ms"

n_servers=$(echo "$SERVERS" | wc -w)
for ((r = 1; r <= ROUNDS; r++)); do
    # Rotate rather than reverse: with three servers, reversing would keep the
    # middle one in the middle forever.
    shift_by=$(( (r - 1) % n_servers ))
    order=$(echo "$SERVERS" | tr ' ' '\n' | awk -v s="$shift_by" '{a[NR]=$0} END {for(i=0;i<NR;i++) print a[((i+s)%NR)+1]}')
    for entry in $order; do
        bin=${entry%%:*}; port=${entry##*:}
        echo "$r $bin $(measure "$bin" "$port" "$CONNS")"
    done
done
