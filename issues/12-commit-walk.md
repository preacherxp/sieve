# 12 Commit walk

Type: AFK

Status (2026-09-30): implemented as `sieve replay --walk [--plant]`. Tested on a local history of
`projects/records` (records carried forward, a deleting commit cleans, missed failures exit 1).
Not yet run on commons-text: cloning it was not permitted in the implementing session.

## Parent PRD

[00](00-prd-local-first-test-records.md)

## What to build

Final evidence based on edits that nobody hand-picked. A new replay mode applies a project's last N
first-parent commits in order to one working tree, carrying test records and build output forward.
It cleans between commits only when issue 09's triggers apply.

For each commit it runs `sieve run` and then a native full build as the reference, plants bugs as in
issue 11, and reports feedback time, the phase breakdown, and missed failures. The existing cold
replay stays as it is.

## Acceptance criteria

- [ ] The walk runs on a public repository, such as the commons-text window already used in
      `docs/performance.md`. It reports per-commit and total feedback time next to the native full
      build.
- [ ] Any missed failure makes the command exit with 1.
- [ ] Records and build output carry forward, and a commit that deletes files triggers `clean`.

## Blocked by

- 09
- 11

## User stories addressed

- User story 20
