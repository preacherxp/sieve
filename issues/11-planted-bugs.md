# 11 Planted bugs

Type: AFK

Status (2026-09-30): implemented as `sieve catalog --plant` and `sieve replay --walk --plant`.
The broken-selector hook is `SIEVE_BROKEN_SELECTOR=1`. Edits are restored after every planted
bug; `--in-place` runs keep a journal that the next run replays after an interruption.

## Parent PRD

[00](00-prd-local-first-test-records.md)

## What to build

Safety evidence on real code, where history rarely contains a failing commit. For each catalog edit,
and later for each commit in the commit walk, Sieve plants a bug in a changed method body, for example
by negating a condition, returning a default value, or dropping a call. It then runs the full suite
and the selected run, and reports a missed failure whenever the full run fails and the selected run
does not. Planted bugs never persist in the working tree.

## Acceptance criteria

- [ ] Planted-bug runs work on `samples/bookstore` and `projects/records`, including context tests.
- [ ] A deliberately broken selector (a test hook) is caught as a missed failure, and the command
      exits with 1.
- [ ] The working tree is restored after every planted bug, including after an interruption.

## Blocked by

- 01
- 04

## User stories addressed

- User story 19
