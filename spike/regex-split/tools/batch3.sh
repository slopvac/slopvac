#!/bin/sh
# SPIKE batch 3 (dev, under slopvac-heavy): timings. Refuses to start above load 12.
# Usage: sh spike/regex-split/tools/batch3.sh <b2-sha8> <set: small|large|e2e> [modes]
set -u
B2=$1
SET=$2
MODES=${3:-"interp jit callout rust"}
WT=$PWD
SHA8=$(git rev-parse --short=8 HEAD)
S=$HOME/tmp/spike-regex-build
D=$S/data
IN=$HOME/personal/dev/slopvac/.git/runs/spike-out-$B2
OUT=$HOME/personal/dev/slopvac/.git/runs/spike-t-$SHA8-$SET
BENCH=$S/bench-$B2
mkdir -p "$OUT"
echo "toolchain=$(rustc --version)" > "$OUT/toolchain.txt"
load1() { cut -d' ' -f1 /proc/loadavg; }
L0=$(load1)
echo "start uptime: $(uptime) nproc=$(nproc)" | tee "$OUT/load.txt"
if awk "BEGIN{exit !($L0 > 12)}"; then
  echo "load-gate: load $L0 above 12, nothing measured" | tee -a "$OUT/load.txt"
  exit 75
fi

# run <mode> <class-label> <patterns> <input> [extra bench args]
run() {
  mode=$1; cls=$2; pats=$3; input=$4; shift 4
  label="$mode/$cls"
  echo "$(date -u +%T) $label load=$(load1)" >> "$OUT/load.txt"
  while :; do
    ( ulimit -v 8388608; SLOPVAC_SPIKE_ENGINE=$mode timeout 7200 "$BENCH" "$pats" "$input" "$label" "$OUT/bench.jsonl" "$@" ) \
      > /dev/null 2>> "$OUT/bench.err"
    rc=$?
    [ $rc = 0 ] && break
    [ $rc = 3 ] && continue
    # Any other exit: memory cap (abort) or the outer timeout. Mark the pattern in flight.
    if [ -f "$OUT/bench.jsonl.cur" ]; then
      cur=$(cut -f1 "$OUT/bench.jsonl.cur")
      origin=$(cut -f3 "$OUT/bench.jsonl.cur")
      printf '{"id":"%s","label":"%s","mode":"%s","origin":"%s","killed":"exit %s (8192 MiB address-space cap or outer timeout)"}\n' \
        "$cur" "$label" "$mode" "$origin" "$rc" >> "$OUT/bench.jsonl"
      rm -f "$OUT/bench.jsonl.cur"
    else
      echo "$label rc=$rc with no pattern in flight" >> "$OUT/bench.err"; break
    fi
  done
}

CORE=$IN/patterns.jsonl
PLUG=$IN/plugin-patterns.jsonl
case $SET in
small)
  for mode in $MODES; do
    for c in block-1m sentence-1m nest-paren-1k nest-paren-10k nest-mixed-1k nest-mixed-10k \
      nest-list-1k nest-list-10k run-ws-1m run-dots-1m run-dash-1m run-a-1m run-word-1m \
      run-caps-1m run-quote-1m run-apos-1m nearmiss-1m; do
      run "$mode" "$c" "$CORE" "$D/inputs/$c.txt"
    done
    for a in native 64k 1m; do
      run "$mode" "recheck-$a" "$CORE" "$D/inputs/block-1m.txt" --pattern-input "$D/attack-core-$a"
    done
    for a in 30 64k 1m; do
      run "$mode" "plugin-$a" "$PLUG" "$D/inputs/block-1m.txt" --pattern-input "$D/attack-$a"
    done
  done ;;
large)
  for c in block-8m sentence-8m nearmiss-8m block-64m sentence-64m; do
    for mode in $MODES; do
      run "$mode" "$c" "$CORE" "$D/inputs/$c.txt"
    done
  done ;;
e2e)
  P=$S/project
  M="python3 $WT/spike/regex-split/tools/measure.py"
  cd "$P"
  for set in docs eval; do
    for rep in 1 2 3; do
      $M "$OUT/e2e.jsonl" "main-interp/$set" 300 16384 -- "$S/slopvac-main-$B2" lint "$set" --format json
      SLOPVAC_SPIKE_ENGINE=interp $M "$OUT/e2e.jsonl" "spike-interp/$set" 300 16384 -- "$S/slopvac-spike-$B2" lint "$set" --format json
      SLOPVAC_SPIKE_ENGINE=split SLOPVAC_SPIKE_ALLOW=$IN/allow.txt $M "$OUT/e2e.jsonl" "split/$set" 300 16384 -- "$S/slopvac-spike-$B2" lint "$set" --format json
      SLOPVAC_SPIKE_ENGINE=jit $M "$OUT/e2e.jsonl" "jit/$set" 300 16384 -- "$S/slopvac-spike-$B2" lint "$set" --format json
      SLOPVAC_SPIKE_ENGINE=callout $M "$OUT/e2e.jsonl" "callout/$set" 300 16384 -- "$S/slopvac-spike-$B2" lint "$set" --format json
    done
  done
  # Findings equal? One JSON report per engine (eval corpus), compared by digest.
  for e in interp split jit callout; do
    SLOPVAC_SPIKE_ENGINE=$e SLOPVAC_SPIKE_ALLOW=$IN/allow.txt timeout 300 "$S/slopvac-spike-$B2" lint eval docs --format json > "$OUT/report-$e.json" 2>/dev/null
    echo "$e rc=$? sha=$(sha256sum < "$OUT/report-$e.json" | cut -c1-16)" >> "$OUT/reports.txt"
  done ;;
esac
echo "end uptime: $(uptime)" | tee -a "$OUT/load.txt"
L1=$(load1)
if awk "BEGIN{exit !($L1 > 12)}"; then echo "load-gate: end load $L1 above 12, DISCARD" | tee -a "$OUT/load.txt"; fi
echo done-$SET
