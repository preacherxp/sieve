# 19 Gradle developer loop

Type: AFK

Status (2026-10-01): implemented: daemon kept, never `clean`, `impactTests` by default, no-change fast path.

## Parent PRD

[15](15-prd-local-mode-for-gradle-and-java-17.md)

## What to build

Remove the fixed costs Sieve itself adds on Gradle, mirroring issue 09.

- Local mode runs on the Gradle daemon; `--no-daemon` stays for static selection only.
- No `clean`: Gradle's incremental compilation removes the outputs of deleted sources itself.
- The default command runs every `Test` task of the project, not `check`, so linters such as
  spotless and detekt do not run on every edit. Arguments after `--` that name tasks replace the
  default.
- When nothing changed since the last passing run with the same invocation, `sieve run` reports
  `NONE` without starting Gradle.
- Document the service-side causes of slow Gradle starts found during measurement, such as starting
  a container during configuration, in the local-mode adoption guide.

## Acceptance criteria

- [ ] A rerun without edits starts no Gradle process.
- [ ] A body edit starts Gradle once, without `clean`, and runs no linter by default.
- [ ] Deleting a source file leaves no stale class that a test can load.
- [ ] `sieve run -- check` runs `check`.

## Blocked by

- 18

## User stories addressed

- User stories 3, 4, 6
