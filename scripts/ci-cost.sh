#!/usr/bin/env bash
# Read-only timing report. Accept a saved GitHub jobs response for reproducible checks.
set -euo pipefail
if [[ ${1:-} == --input ]]; then
  cat "${2:?jobs JSON path required}"
else
  run=${1:?GitHub run ID required}
  [[ $run =~ ^[0-9]+$ ]] || { echo 'Run ID must be numeric' >&2; exit 2; }
  gh api --paginate "repos/${GITHUB_REPOSITORY:?}/actions/runs/$run/jobs?per_page=100"
fi | jq --slurp '
  [.[].jobs[] | select(.status == "completed")] as $completed |
  [$completed[] | select(.started_at != null and .completed_at != null) |
    {name, conclusion, start: (.started_at | fromdateiso8601), end: (.completed_at | fromdateiso8601),
     steps: [.steps[]? | select(.started_at != null and .completed_at != null) |
       {name, conclusion, seconds: ((.completed_at | fromdateiso8601) - (.started_at | fromdateiso8601))}]}] as $jobs |
  if ($jobs | length) == 0 then error("No completed jobs") else
    {completed_jobs: ($completed | length),
     timed_jobs: ($jobs | length),
     aggregate_runner_seconds: ([$jobs[] | .end - .start] | add),
     observed_wall_seconds: (($jobs | map(.end) | max) - ($jobs | map(.start) | min)),
     jobs: [$jobs[] | {name, conclusion, seconds: (.end - .start), steps}]}
  end'
