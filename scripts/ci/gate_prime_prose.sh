#!/usr/bin/env bash
# Lint the guidance text that `slopvac prime write` and `slopvac prime review` render.
#
# Usage: gate_prime_prose.sh <slopvac-binary> <v3-config> <output-dir>
#
# The output directory must be a missing path or an empty directory that you can list and
# write. The script refuses anything else with exit 2: a symlink (even a dangling one), a
# file, a directory it cannot inspect and a non-empty directory. It never deletes a file, and
# `noclobber` makes a write fail rather than truncate a file that appears during the run.
#
# The script renders every conditional branch of the two topics, then lints the files with
# the 3.x binary. Exit 0 passes, exit 1 means a gate failed and exit 2 means the run was
# incomplete. Every nonzero lint exit fails the script, so an incomplete run never passes.
#
# Branches and the render that covers each one (tests/test_prime_prose_gate.py checks this):
#   write.md.j2   level minimal / default / full      write-minimal, write-ask, write-full
#                 review control off / ask / auto     write-minimal, write-ask, write-full
#   review.md.j2  review_type auto / mechanical /     review-<type>-<depth>
#                   update / rewrite
#                 review_depth self / subagent /      review-<type>-<depth>
#                   adversary
#                 level != minimal (verdict block)    review-auto-subagent-minimal and the rest
#                 review_approval auto                not reachable: `prime` never sets it
set -euo pipefail
# Never overwrite a file that exists: a write into the output directory fails instead.
set -o noclobber

refuse() {
  echo "gate_prime_prose.sh: $1" >&2
  exit 2
}

[ "$#" -eq 3 ] || refuse "usage: gate_prime_prose.sh <slopvac-binary> <v3-config> <output-dir>"
bin=$(realpath "$1")
config=$(realpath "$2")
out=$3

# Use only a missing path or an empty directory that this user can list, search and write.
# Fail closed: a path the script cannot inspect is refused. A symlink is refused even when
# it dangles or points at an empty directory, because the script would write through it.
if [ -L "$out" ]; then
  refuse "$out is a symlink"
elif [ -e "$out" ]; then
  [ -d "$out" ] || refuse "$out exists and is not a directory"
  [ -r "$out" ] && [ -w "$out" ] && [ -x "$out" ] || refuse "$out is not readable, writable and searchable"
  entries=$(ls -A -- "$out") || refuse "cannot list $out"
  [ -z "$entries" ] || refuse "$out is not empty"
else
  # `mkdir` without -p fails if the path appears meanwhile.
  mkdir -p -- "$(dirname -- "$out")"
  mkdir -- "$out" || refuse "cannot create $out"
fi
out=$(realpath "$out")

# Render `prime <topic> [flags]` to <name>.md with [guidance] values from the remaining lines.
render() {
  local name=$1 topic=$2 flags=$3 guidance=$4
  mkdir -p "$out/work/$name"
  {
    cat "$config"
    printf '\n[guidance]\n%s\n' "$guidance"
  } > "$out/work/$name/slopvac.toml"
  # shellcheck disable=SC2086 # the flags are a fixed word list
  (cd "$out/work/$name" && "$bin" prime "$topic" $flags) > "$out/$name.md"
}

render write-minimal write "--level minimal" 'review = "off"'
render write-ask write "--level default" 'review = "ask"'
render write-full write "--level full" 'review = "auto"'

for type in mechanical update rewrite; do
  for depth in self subagent adversary; do
    render "review-$type-$depth" review "--type $type" "review_depth = \"$depth\""
  done
done
for depth in self subagent adversary; do
  render "review-auto-$depth" review "" "review_depth = \"$depth\""
done
render review-auto-subagent-minimal review "--level minimal" 'review_depth = "subagent"'

cat "$config" > "$out/slopvac.toml"
cd "$out"
"$bin" lint ./*.md
