#!/bin/sh
# SPIKE: run timing stages on dev one after another, each only when the 1-minute load is under
# GATE (default 4). Light: sleeps and polls the load and the stage logs, runs nothing heavy.
# Each stage runs through slopvac-heavy. A stage that refuses (exit 75, load gate) or ends
# DISCARD is retried while its window lasts (WINDOW seconds, default 3600, per stage).
# Usage (from the dev worktree): setsid nohup sh spike/regex-split/tools/gate_run.sh <stage...>
set -u
GATE=${GATE:-4}
WINDOW=${WINDOW:-3600}
WT=$PWD
SHA8=$(git rev-parse --short=8 HEAD)
R=$HOME/personal/dev/slopvac/.git/runs
G=$R/spike-gate-$SHA8.log
load1() { cut -d' ' -f1 /proc/loadavg; }
under() { awk "BEGIN{exit !($1 < $GATE)}"; }
echo "gate=$GATE window=$WINDOW stages=$* start=$(date -u +%FT%TZ)" >> "$G"
for stage in "$@"; do
  t0=$(date +%s)
  try=0
  while :; do
    if [ $(( $(date +%s) - t0 )) -gt "$WINDOW" ]; then
      echo "$(date -u +%T) $stage: no quiet window in ${WINDOW}s, STOP" >> "$G"
      echo "gate-rc=76" >> "$G"
      exit 76
    fi
    l=$(load1)
    if ! under "$l"; then sleep 30; continue; fi
    try=$((try + 1))
    name=spike-t-$stage-$SHA8-$try
    echo "$(date -u +%T) $stage try $try at load $l -> $name" >> "$G"
    slopvac-heavy "$name" "$WT" "GATE=$GATE sh spike/regex-split/tools/batch3.sh $stage" > /dev/null
    until grep -q '^rc=' "$R/$name.log" 2>/dev/null; do sleep 20; done
    rc=$(sed -n 's/^rc=//p' "$R/$name.log" | tail -1)
    if [ "$rc" = 75 ]; then echo "$(date -u +%T) $stage refused (load gate)" >> "$G"; sleep 60; continue; fi
    if grep -q DISCARD "$R/$name.log"; then echo "$(date -u +%T) $stage DISCARD (end load), retry" >> "$G"; continue; fi
    echo "$(date -u +%T) $stage done rc=$rc" >> "$G"
    break
  done
done
echo "gate-rc=0" >> "$G"
