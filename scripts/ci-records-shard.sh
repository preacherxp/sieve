#!/usr/bin/env bash
# Runs one shard of the native test-record checks (tests/records.rs). The named shards hold
# the slowest tests; `rest` skips exactly those names, so every test runs in exactly one
# shard, including tests added later. A named test that no longer exists fails the shard.
set -euo pipefail
: "${SHARD:?}"
walks=(
  catalog_times_edits_and_planted_bugs_catch_a_broken_selector
  commit_walk_carries_records_forward_and_cleans_after_deletions
  container_reuse_keeps_containers_between_runs
)
selection=(
  records_skip_unchanged_tests_and_rerun_what_changed
  records_cover_shared_state_annotations_and_context_structure
  dependency_bumps_rerun_the_tests_that_used_the_bumped_jar
  gradle_version_bumps_rerun_only_the_tests_that_ran_the_library
  maven_version_bumps_rerun_only_the_tests_that_ran_the_library
  local_records_invalidate_properties_environment_and_preserved_metadata_edits
)
test=(cargo +"${RUST_TOOLCHAIN:-1.92.0}" test --locked --test records --)
listed=$("${test[@]}" --list --include-ignored | sed -n 's/: test$//p')
for name in "${walks[@]}" "${selection[@]}"; do
  if ! grep -qx -- "$name" <<<"$listed"; then
    echo "Shard test $name does not exist; update scripts/ci-records-shard.sh" >&2
    exit 2
  fi
done
args=()
case "$SHARD" in
  walks) args=("${walks[@]}") ;;
  selection) args=("${selection[@]}") ;;
  rest)
    for name in "${walks[@]}" "${selection[@]}"; do args+=(--skip "$name"); done
    ;;
  *) echo "Unknown shard: $SHARD" >&2; exit 2 ;;
esac
"${test[@]}" --include-ignored --test-threads=1 --exact "${args[@]}"
