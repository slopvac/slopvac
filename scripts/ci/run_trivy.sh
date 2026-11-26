#!/usr/bin/env bash
# Run one Trivy filesystem scan with a fixed policy and print nothing raw.
#
# Trivy runs here, not inside trivy-action, so its stdout and stderr can go to files.
# Scanned content (a file name, a package name, a path in an error) must never reach
# the job log unescaped, because the runner reads a leading double colon as a command.
# The policy comes from the caller as fixed workflow text. The config, the secret
# config and the cache sit under RUNNER_TEMP, never in the checkout, so a change under
# review cannot plant them.
#
# usage: run_trivy.sh SCANNERS SEVERITIES SARIF_PATH
set -uo pipefail

scanners="${1:?usage: run_trivy.sh SCANNERS SEVERITIES SARIF_PATH}"
severities="${2:?usage: run_trivy.sh SCANNERS SEVERITIES SARIF_PATH}"
sarif="${3:?usage: run_trivy.sh SCANNERS SEVERITIES SARIF_PATH}"
temp="${RUNNER_TEMP:?RUNNER_TEMP is not set}"
here="$(dirname "${BASH_SOURCE[0]}")"
logs="$temp/trivy-logs"

mkdir -p "$logs"
status=0
trivy fs . \
  --scanners "$scanners" \
  --severity "$severities" \
  --exit-code 1 \
  --format sarif \
  --output "$sarif" \
  --ignorefile .trivyignore.yaml \
  --config "$temp/trivy-trusted/trivy.yaml" \
  --secret-config "$temp/trivy-trusted/trivy-secret.yaml" \
  --cache-dir "$temp/trivy-cache" \
  --quiet \
  >"$logs/stdout.txt" 2>"$logs/stderr.txt" || status=$?

if [ "$status" -ne 0 ]; then
  echo "Trivy scan failed (exit $status)"
  python3 "$here/../print_trivy_findings.py" "$sarif" || true
fi
exit "$status"
