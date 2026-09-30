# 09 Incremental local builds and the no-change fast path

Type: AFK

Status (2026-09-30): implemented; the Maven start count is checked with a counting wrapper
in `tests/records.rs`. `build-info` is not skipped after all: Spring Boot creates its
`BuildProperties` bean only when the file exists, so skipping it changes application behavior.
Its `build.time` line is left out of the file hash instead.

## Parent PRD

[00](00-prd-local-first-test-records.md)

## What to build

Stop paying for cold builds on a developer machine.

- In local mode, `sieve run` runs an incremental `mvn verify`. It adds `clean` only when stale output
  is possible: a deletion or rename since the last run, an edit to the POM or another build input, or
  an edit to a `generated` input.
- Spring Boot `repackage` and `build-info` are skipped, because no test uses them.
- When nothing changed since the last passing run, or only ignored paths changed, Sieve reports `NONE`
  without starting Maven.

## Acceptance criteria

- [ ] A rerun without edits returns without starting any build-tool process.
- [ ] Deleting a resource or source file triggers `clean` for that run. A stale-output scenario (a
      deleted resource that a test would still find without `clean`) fails as expected.
- [ ] A body edit runs without `clean` and without repackaging.
- [ ] Maven starts at most once per run in every case.
- [ ] `sieve run -- <goals>` overrides the default command.

## Blocked by

- 04

## User stories addressed

- User stories 1, 4, 15
