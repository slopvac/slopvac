set shell := ["bash", "-cu"]

# Dispatch stable publishing for an existing version tag; approve it in GitHub.
release VERSION:
    #!/usr/bin/env bash
    set -euo pipefail
    version={{quote(VERSION)}}
    if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        echo "expected stable version X.Y.Z, got $version" >&2
        exit 2
    fi
    tag="v$version"
    if ! git ls-remote --exit-code --refs origin "refs/tags/$tag" >/dev/null; then
        echo "tag $tag does not exist on origin" >&2
        exit 1
    fi
    run_url="$(gh workflow run publish-lint.yml --ref "$tag" -f dry-run=false -f target=pypi)"
    printf '%s\nRun waits for the pypi environment approval.\n' "$run_url"

# Wait for a normal system load before measuring.
perf-wait-idle:
    uv run --group profile python scripts/perf/perf.py wait-idle

# Benchmark all scenarios or the selected SCENARIO; retain Hyperfine exports.
perf-bench $SCENARIO="": perf-wait-idle
    PERF_SCENARIO="${SCENARIO#SCENARIO=}" uv run --group profile python scripts/perf/perf.py bench --idle-checked

# Profile Python import time; PERF_OPEN=1 opens Tuna interactively.
perf-import $SCENARIO="": perf-wait-idle
    PERF_SCENARIO="${SCENARIO#SCENARIO=}" uv run --group profile python scripts/perf/perf.py import --idle-checked

# Capture a pyinstrument text and HTML profile.
perf-cpu $SCENARIO="": perf-wait-idle
    PERF_SCENARIO="${SCENARIO#SCENARIO=}" uv run --group profile python scripts/perf/perf.py cpu --idle-checked

# Capture Scalene JSON and its CLI rendering.
perf-scalene $SCENARIO="": perf-wait-idle
    PERF_SCENARIO="${SCENARIO#SCENARIO=}" uv run --group profile python scripts/perf/perf.py scalene --idle-checked

# Capture a VizTracer JSON timeline.
perf-timeline $SCENARIO="": perf-wait-idle
    PERF_SCENARIO="${SCENARIO#SCENARIO=}" uv run --group profile python scripts/perf/perf.py timeline --idle-checked

# Capture Memray stats and an HTML flamegraph.
perf-mem $SCENARIO="": perf-wait-idle
    PERF_SCENARIO="${SCENARIO#SCENARIO=}" uv run --group profile python scripts/perf/perf.py mem --idle-checked

# Record py-spy speedscope output with sudo (may prompt for a password).
perf-spy $SCENARIO="": perf-wait-idle
    PERF_SCENARIO="${SCENARIO#SCENARIO=}" uv run --group profile python scripts/perf/perf.py spy --idle-checked

# Time each line of FUNCS (space-separated module:qualname) without decorators.
perf-lines $FUNCS $SCENARIO="": perf-wait-idle
    PERF_LINE_FUNCS="${FUNCS#FUNCS=}" PERF_SCENARIO="${SCENARIO#SCENARIO=}" uv run --group profile python scripts/perf/perf.py lines --idle-checked

# Run the benchmark and all profilers in sequence after one idle gate.
perf $SCENARIO="": perf-wait-idle
    PERF_SCENARIO="${SCENARIO#SCENARIO=}" uv run --group profile python scripts/perf/perf.py all --idle-checked

# Compare BASE and HEAD in one interleaved run; report unless PERF_FAIL=1.
# Pass `BASE=<ref>` and `SCENARIO=<name>` in either order; a bare first value is BASE.
perf-check $BASE="" $SCENARIO="": perf-wait-idle
    #!/usr/bin/env bash
    set -euo pipefail
    base="" scenario=""
    for value in "$BASE" "$SCENARIO"; do
        case "$value" in
            BASE=*) base="${value#BASE=}" ;;
            SCENARIO=*) scenario="${value#SCENARIO=}" ;;
            "") ;;
            *) if [ -z "$base" ]; then base="$value"; else scenario="$value"; fi ;;
        esac
    done
    PERF_BASE="$base" PERF_SCENARIO="$scenario" uv run --group profile python scripts/perf/perf.py check --idle-checked

# Validate the v3 contract schemas and conformance corpus offline.
contract:
    uv run --extra dev pytest tests/contract

# Generate the reference docs site source into build/site-src.
docs:
    uv run python scripts/docs/build_site.py

# Local check: build/site-src matches a fresh build, links resolve, schema ids match. Changes nothing.
docs-check:
    uv run python scripts/docs/build_site.py --check

# Build rustdoc for every workspace crate with warnings denied.
doc-rust:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items

# Lint the eval corpus with ENGINE ('python' or a slopvac command) and store OUT.
eval-run ENGINE="python" OUT="evals/.cache/results/latest.json":
    uv run python -m evals run --engine {{quote(ENGINE)}} --out {{quote(OUT)}}

# Offline tier: score stored results without starting an engine.
eval-score RESULTS SPLIT="all":
    uv run python -m evals score {{quote(RESULTS)}} --split {{quote(SPLIT)}}

# Paired A/B of two stored results; exits 1 when B regresses beyond noise.
eval-ab A B SPLIT="all":
    uv run python -m evals ab {{quote(A)}} {{quote(B)}} --split {{quote(SPLIT)}} --fail-on-regression
