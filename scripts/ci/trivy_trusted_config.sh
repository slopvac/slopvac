#!/usr/bin/env bash
# Write the trusted Trivy config for one job, outside the checkout.
#
# Trivy reads ./trivy.yaml and ./trivy-secret.yaml from the working directory unless
# told otherwise, so a change under review could plant either file. The caller passes
# the severity policy as fixed workflow text, never a value from the pull request.
#
# usage: trivy_trusted_config.sh SEVERITIES    (a comma list such as HIGH,CRITICAL)
set -euo pipefail

severities="${1:?usage: trivy_trusted_config.sh SEVERITIES}"
dir="${RUNNER_TEMP:?RUNNER_TEMP is not set}/trivy-trusted"

mkdir -p "$dir"
{
  echo "severity:"
  IFS=, read -ra levels <<<"$severities"
  for level in "${levels[@]}"; do
    case "$level" in
      UNKNOWN | LOW | MEDIUM | HIGH | CRITICAL) echo "  - $level" ;;
      *) echo "unknown severity: $level" >&2; exit 2 ;;
    esac
  done
} >"$dir/trivy.yaml"
echo '{}' >"$dir/trivy-secret.yaml"
