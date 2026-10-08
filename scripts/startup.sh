#!/bin/zsh
# Measures startup: launches APP RUNS times (default 3) the way a user does
# (with `open`, the window in the background), and prints when each step
# happened, in seconds since `main`: window shown, Home requested, fetched,
# shown, first thumbnail shown.
#
#   scripts/startup.sh dist/Kilo.app [RUNS]
set -eu
cd "$(dirname "$0")/.."
APP=${1:?app}; RUNS=${2:-3}
EXE="$PWD/$APP/Contents/MacOS/Kilo"
[ -x "$EXE" ] || EXE="$APP/Contents/MacOS/Kilo"
OUT=$(mktemp -d)
printf '%-4s %8s %8s %8s %8s %8s\n' run window request fetched shown image
for r in $(seq 1 $RUNS); do
    LOG=$OUT/run$r.log
    open -g -n -o $LOG --stderr $LOG --env KILO_NO_ACTIVATE=1 --env KILO_SCENARIO=wait:4 "$APP"
    until grep -q "scenario done" $LOG 2>/dev/null; do sleep 0.2; done
    pkill -f "^$EXE\$" || true
    at() { grep -m1 "$1" $LOG | sed 's/^\[ *\([0-9.]*\)s\].*/\1/'; }
    printf '%-4s %8s %8s %8s %8s %8s\n' $r "$(at 'ui: window shown')" "$(at 'page: Home requested')" "$(at 'page: fetched')" "$(at 'page: shown')" "$(at 'images: first shown')"
    sleep 1
done
echo "(logs in $OUT)"
