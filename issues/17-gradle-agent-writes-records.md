# 17 Gradle: the agent writes test records

Type: AFK

Status (2026-10-01): implemented with `projects/single-gradle` instead of a new `projects/records-gradle` fixture (no Spring Boot or second `Test` task yet). Records key on the class name only; a class in two `Test` tasks shares one record.

## Parent PRD

[15](15-prd-local-mode-for-gradle-and-java-17.md)

## What to build

The thinnest end-to-end path on Gradle, mirroring issue 03. A single-module Gradle project opts in
with `sieve run --records` or `"records": true`, Gradle starts once, and every test class leaves a
test record in `.sieve/`. No test is skipped yet.

- The init script adds the agent to every `Test` task through `jvmArgumentProviders`, only when
  `sieve run` passes the records property. The Gradle daemon and the Kotlin compile daemon never see
  the agent.
- Failure status comes from each `Test` task's JUnit XML under `build/test-results/<task>/`. Decide
  whether a record key needs the task name, since one class can run in more than one `Test` task.
- `maxParallelForks > 1` writes records from several JVMs through the existing record-store lock.
- A new fixture, `projects/records-gradle`, mirrors `projects/records` in Kotlin and Java: plain,
  resource-reading, and Spring Boot context tests, plus a second `Test` task for integration tests.
  Run it on a Java 24+ test JVM until issue 16 lands.

## Acceptance criteria

- [ ] Two runs of `projects/records-gradle` produce records listing exactly the project methods each
      test class executed, with hashes, JDK version, and outcome, for both `Test` tasks.
- [ ] `sieve run` starts Gradle exactly once in this mode, checked with a counting wrapper.
- [ ] Records from two parallel forks are complete and uncorrupted.
- [ ] Plain `./gradlew test` without Sieve runs unchanged and writes no records.

## Blocked by

None - can start immediately.

## User stories addressed

- User stories 1, 3, 7
