#!/bin/sh
# Launch the app on a file, save a screenshot of the window, then quit.
# usage: scripts/shot.sh input.pdf out.png [extra env assignments...]
set -e
in="$1"; out="$2"; shift 2
env REFLOW_SCREENSHOT="$out" REFLOW_EXIT=1 "$@" ./target/debug/reflow "$in" &
pid=$!
( sleep 40; kill $pid 2>/dev/null ) &
wait $pid
