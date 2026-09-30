# 08 Static fallback for tests without a record

Type: AFK

Status (2026-09-30): implemented; needs no `build_fingerprint` for a single module. Tested
with records deleted, with and without `--base`.

## Parent PRD

[00](00-prd-local-first-test-records.md)

## What to build

A test class without a passing record (a fresh clone, a deleted `.sieve/`, a new test) currently
always runs. When `--base` is given, Sieve also skips it if today's static class-level analysis shows
that it reaches no change since the base, which is assumed green. Either kind of evidence is enough to
skip a test, and `--output` says which one applied.

## Acceptance criteria

- [ ] With `.sieve/` deleted and `--base` given, a run selects exactly what today's class-level
      selection selects for the same change.
- [ ] Without `--base`, tests without a record run.
- [ ] Oracle scenarios pass with records absent, partial, and complete.

## Blocked by

- 04

## User stories addressed

- User stories 11, 21
