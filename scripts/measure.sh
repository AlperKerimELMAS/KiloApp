#!/bin/zsh
# Measures a Kilo.app the way docs/PLAN.md does: launched with `open` (so
# macOS charges its XPC services to it), driven by KILO_SCENARIO, then
# sampled with kilo-probe.
#
#   scripts/measure.sh APP SCENARIO [SECONDS] [--env K=V ...]
#       Runs SCENARIO, then samples the process tree for SECONDS (default
#       10) and prints footprint, CPU, per-process breakdown and memory
#       categories.
#   scripts/measure.sh APP --play VIDEO_ID [SECONDS] [--env K=V ...]
#       Plays VIDEO_ID, lets it settle for 25 s, samples for SECONDS (default
#       60) and also counts bytes received by the helper's WebKit
#       networking process. Plays audio on the signed-in account.
#
# Examples:
#   scripts/measure.sh dist/Kilo.app wait:15                       # window open, Home
#   scripts/measure.sh dist/Kilo.app wait:12,close,wait:20         # window closed
#   scripts/measure.sh dist/Kilo.app --play lYBUbBu4W08            # playing
set -eu
cd "$(dirname "$0")/.."
APP=${1:?app}; shift
PROBE=target/release/kilo-probe
[ -x $PROBE ] || cargo build --release -p kilo-probe
if [ "${1:-}" = "--play" ]; then
    VID=$2; SECS=${3:-60}; shift 3 2>/dev/null || shift $#
    SCEN="wait:1,play:$VID,wait:25"
else
    SCEN=$1; SECS=${2:-10}; shift 2 2>/dev/null || shift $#
    VID=
fi
OUT=$(mktemp -d)
EXE="$PWD/$APP/Contents/MacOS/Kilo"
[ -x "$EXE" ] || EXE="$APP/Contents/MacOS/Kilo"
open -g -n -o $OUT/log --stderr $OUT/log --env KILO_NO_ACTIVATE=1 --env "KILO_SCENARIO=$SCEN" "$@" "$APP"
P=
for i in $(seq 1 40); do P=$(pgrep -f "^$EXE\$" | head -1 || true); [ -n "$P" ] && break; sleep 0.25; done
[ -n "$P" ] || { echo "Kilo didn't start"; exit 1; }
until grep -q "scenario done" $OUT/log 2>/dev/null; do sleep 0.5; done
if [ -n "$VID" ]; then
    NET=$(pgrep -n -f com.apple.WebKit.Networking)
    B0=$(nettop -P -L 1 -n -x -J bytes_in -p $NET | tail -1 | cut -d, -f2)
fi
$PROBE $P -d $SECS -i 2 --breakdown > $OUT/probe.txt
footprint -p $P > $OUT/footprint.txt 2>&1 || true
if [ -n "$VID" ]; then
    B1=$(nettop -P -L 1 -n -x -J bytes_in -p $NET | tail -1 | cut -d, -f2)
    echo "network: $(( (B1 - B0) * 60 / SECS / 1024 )) KB/min"
fi
kill $P 2>/dev/null || true
grep -A20 summary $OUT/probe.txt
sed -n '/Category/,/TOTAL/p' $OUT/footprint.txt | head -12
echo "(log and raw output in $OUT)"
