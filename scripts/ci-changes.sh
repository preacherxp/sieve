#!/usr/bin/env bash
# Prints `code=true` unless every changed path is documentation that no check reads, and
# `code=false` otherwise; the JVM jobs run only for `code=true`. Fixture and sample READMEs
# are test inputs and count as code. Unknown bases and missing history count as code.
set -euo pipefail
EVENT_NAME=${EVENT_NAME:-}
PR_BASE=${PR_BASE:-}
PUSH_BASE=${PUSH_BASE:-}
base=
if [[ "$EVENT_NAME" == pull_request ]]; then
  base=$PR_BASE
elif [[ "$EVENT_NAME" == push && "$PUSH_BASE" != 0000000000000000000000000000000000000000 ]]; then
  base=$PUSH_BASE
fi
if [[ -z "$base" ]] || ! paths=$(git diff --name-only -z --end-of-options "$base" HEAD -- 2>/dev/null | tr '\0' '\n'); then
  echo code=true
  exit 0
fi
code=false
while IFS= read -r path; do
  case "$path" in
    '') ;;
    */*.md | */*.adoc) case "$path" in docs/* | site/* | assets/* | issues/*) ;; *) code=true ;; esac ;;
    *.md | *.adoc | docs/* | site/* | assets/* | issues/*) ;;
    *) code=true ;;
  esac
done <<<"$paths"
echo "code=$code"
