# 20 Catalog, planted bugs, and commit walk on Gradle

Type: AFK

Status (2026-10-01): walk and catalog accept Gradle; the native reference runs `impactTests` without the agent. While doing this, planted runs turned out to use a different invocation than normal runs, so every planted run selected everything and missed-failure checks held trivially; all runner calls now share the keep-going flag, and the catalog test asserts the planted run executes fewer tests than the full run.

## Parent PRD

[15](15-prd-local-mode-for-gradle-and-java-17.md)

## What to build

Extend the evidence tools of issues 01, 11, and 12 to Gradle, so Gradle local mode is judged by the
same oracle as Maven.

- `catalog` runs the native reference as Gradle's `Test` tasks without `clean`, and its phase
  breakdown recognizes Gradle output (configuration, compilation, test JVM start, tests).
- `--plant` mutates Kotlin method bodies as well as Java, or reports that an edited Kotlin body was
  not planted instead of passing silently.
- `replay --walk` accepts single-module Gradle projects and carries records and build output forward.

## Acceptance criteria

- [ ] A catalog on `projects/records-gradle` reports per-edit feedback time next to the native run.
- [ ] Planted bugs in Java and Kotlin bodies are caught; `SIEVE_BROKEN_SELECTOR=1` makes them missed
      failures that exit with 1.
- [ ] A walk over a local Gradle history carries records forward and reports missed failures.

## Blocked by

- 18

## User stories addressed

- User story 9
