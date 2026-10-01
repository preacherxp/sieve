# 18 Gradle: skip tests whose records are unchanged

Type: AFK

Status (2026-10-01): implemented for body edits, failures, and `--tests`. Gradle test results are deleted before each run, so `Test` tasks never come back up to date or from the cache with stale outputs. Kotlin inline functions and the issue 04–08 scenarios on Gradle are not covered by tests yet.

## Parent PRD

[15](15-prd-local-mode-for-gradle-and-java-17.md)

## What to build

Turn on the existing `decide` rules (issues 04–08) for Gradle test JVMs. The rules stay in Rust; this
slice supplies the Gradle-specific inputs and confirms they are complete.

- Invocation key: Gradle arguments and task names instead of Maven's; build inputs through the
  existing fingerprint (build scripts, `gradle/`, `buildSrc`, wrapper properties).
- Explicit requests (`--tests …`) are never dropped.
- Kotlin: an inline function has no call at run time; its callers' bodies hold the inlined code.
  Confirm that an inline-function edit changes the recorded hashes of its callers after Kotlin's
  incremental compilation, or treat inline functions as structural.
- Gradle may skip a `Test` task as up to date or restore it from the build cache, which writes no
  records. Confirm that this cannot report a pass for classes or resources the tests never ran
  against; otherwise disable both for `Test` tasks while records are on.

## Acceptance criteria

- [ ] The issue 04–08 scenarios pass on `projects/records-gradle`: body edits, structural changes,
      Spring wiring, context startup, resource reads, and the static fallback.
- [ ] A Kotlin inline-function edit runs every test whose recorded code inlined it.
- [ ] `SIEVE_BROKEN_SELECTOR=1` makes the native tests fail.
- [ ] A test that failed runs again until it passes, in every `Test` task.

## Blocked by

- 17

## User stories addressed

- User stories 2, 5
