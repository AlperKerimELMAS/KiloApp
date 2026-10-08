# Shared by the measuring scripts: launching one Kilo of our own (never
# touching other running copies) and waiting for it with a deadline.

# launch APP LOG [open's arguments...]: launches a new copy of APP the way a
# user does (with `open`, so macOS charges its XPC services to it) and prints
# its pid, or fails after 10 s.
launch() {
    local app=$1 log=$2; shift 2
    local exe="$PWD/$app/Contents/MacOS/Kilo"
    [ -x "$exe" ] || exe="$app/Contents/MacOS/Kilo"
    local before; before=$(pgrep -f "^$exe\$" || true)
    open -g -n -o "$log" --stderr "$log" "$@" "$app"
    local pid
    for _ in $(seq 1 40); do
        for pid in $(pgrep -f "^$exe\$" || true); do
            if ! printf '%s\n' "$before" | grep -qx "$pid"; then
                echo "$pid"
                return 0
            fi
        done
        sleep 0.25
    done
    echo "Kilo didn't start" >&2
    return 1
}

# await LOG PATTERN PID SECONDS: waits until LOG has PATTERN, failing if PID
# exits or SECONDS pass first.
await() {
    local log=$1 pattern=$2 pid=$3 deadline=$(( $(date +%s) + $4 ))
    until grep -q "$pattern" "$log" 2>/dev/null; do
        kill -0 "$pid" 2>/dev/null || { echo "Kilo ($pid) exited before \"$pattern\"" >&2; return 1; }
        [ "$(date +%s)" -lt "$deadline" ] || { echo "no \"$pattern\" within $4 s" >&2; return 1; }
        sleep 0.25
    done
}
