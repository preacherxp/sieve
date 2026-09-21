#!/usr/bin/env bash
set -euo pipefail
: "${BUILD_TOOL:?}" "${MODE:?}"
EVENT_NAME=${EVENT_NAME:-}
PR_BASE=${PR_BASE:-}
PUSH_BASE=${PUSH_BASE:-}
case "$BUILD_TOOL" in maven|gradle) ;; *) echo "Unknown build tool: $BUILD_TOOL" >&2; exit 2 ;; esac
selection_dir=${SELECTION_DIR:-validation-results}
mkdir -p "$selection_dir"
executable=gradle
if [[ "$BUILD_TOOL" == maven ]]; then executable=mvn; fi
args=(--full)
if [[ "$MODE" == selected ]]; then
  if [[ "$EVENT_NAME" == pull_request && -n "$PR_BASE" ]]; then
    args=(--base "$PR_BASE")
  elif [[ "$EVENT_NAME" == push && -n "$PUSH_BASE" && "$PUSH_BASE" != 0000000000000000000000000000000000000000 ]]; then
    args=(--base "$PUSH_BASE")
  fi
elif [[ "$MODE" != full ]]; then
  echo "Unknown test mode: $MODE" >&2
  exit 2
fi
if [[ "$MODE" == selected ]]; then
  "${SIEVE_BIN:-target/release/sieve}" select \
    --workspace "projects/$BUILD_TOOL" \
    --output "$selection_dir/$BUILD_TOOL-selection.json" \
    "${args[@]}"
  if [[ $(jq -r '.mode' "$selection_dir/$BUILD_TOOL-selection.json") == ALL ]]; then
    echo 'Selection is ALL; the full-test stage runs this suite.'
    exit 0
  fi
fi
"${SIEVE_BIN:-target/release/sieve}" run \
  --workspace "projects/$BUILD_TOOL" \
  --executable "$executable" \
  --output "$selection_dir/$BUILD_TOOL-selection.json" \
  "${args[@]}"
