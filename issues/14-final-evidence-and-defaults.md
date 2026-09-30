# 14 Final evidence and defaults

Type: HITL

## Parent PRD

[00](00-prd-local-first-test-records.md)

## What to build

Confirm the goal on the benchmark services and fix the defaults. Run the edit catalog with planted
bugs, and one commit walk, on each of the four services. Decide which speed-ups from issue 10 are on
by default. Publish public-safe conclusions, without internal names, in `docs/performance.md`, and
keep private details private.

## Acceptance criteria

- [ ] Zero missed failures across catalogs, planted bugs, and commit walks.
- [ ] No selected run did more build work than the full run.
- [ ] The weighted total feedback time is reported for each service against today's Sieve and the
      native build.
- [ ] The default for each speed-up is recorded with its measured reason.

## Blocked by

- 02, 05, 06, 07, 08, 09, 10, 11, 12, 13

## User stories addressed

- User stories 4, 18, 19, 20
