# PRD: Local mode for Gradle and Java 17+ test JVMs

Status: proposed 2026-10-01. Decision: [ADR 0003](../docs/adr/0003-local-mode-for-gradle-and-java-17.md),
building on [ADR 0001](../docs/adr/0001-record-runtime-evidence-for-context-tests.md) and
[ADR 0002](../docs/adr/0002-select-tests-inside-the-test-jvm.md). Terms: [CONTEXT.md](../CONTEXT.md).

## Problem

Static selection does not shorten feedback on single-module projects. Measured on 2026-09-30 on two
single-module Gradle/Kotlin Spring Boot services (private results):

- Module selection runs the whole suite for every source edit; only about 2% of 150 commits changed
  nothing but ignored paths.
- Class-level selection was never faster than the full build in nine replayed commits (1,048 s full
  against 1,075 s selected). Every component edit reached every context test (110–152 classes), and
  the second Gradle start, which also started the build's Postgres container again, cost 5–13 s.
- Every static run starts with `clean` and `--no-daemon`, so a developer pays full compilation and
  a cold Gradle start on every run.

Local mode solves these on Maven, but the services build with Gradle and test on Java 17 or 21,
while the agent needs Maven and Java 24+.

## Goal

Minimal feedback time for developers running tests locally on single-module Gradle and Maven
projects with Java 17+ test JVMs.

- Zero missed failures: native record tests, planted bugs, and a commit walk on Gradle must pass.
- One Gradle start per run, on the daemon, without `clean`.
- A method-body edit runs only the test classes that executed the method, including context tests.

## Solution

- The agent instruments classes with a vendored, relocated ASM and runs on Java 17+ test JVMs
  (ADR 0003). Selection and records work as on Maven: a JUnit Platform discovery filter asks
  `sieve decide`, and `sieve record` writes records at JVM exit.
- On Gradle, the init script attaches the agent to every `Test` task when records are enabled.
  `sieve run` runs the project's `Test` tasks once, on the daemon, with Gradle's own incremental
  compilation.
- `catalog`, `--plant`, and `replay --walk` support Gradle, so the same evidence is collected on
  both build tools.

## Scope

In: single-module Gradle and Maven projects, JUnit Platform tests, Java 17+ test JVMs, developer
machines.

Out: CI (static selection and full-suite validation stay as they are), records shared between
machines, multi-module projects, Android, and Kotlin Multiplatform.

## User stories

As a developer on a single-module Gradle service:

1. `sieve run --records` on a Java 17 or 21 test JVM records every test on the first run and then
   runs only what my edits affect.
2. When I edit one method body, only the test classes that executed it run, including context tests.
3. `sieve run` starts Gradle at most once, keeps the daemon, and never adds `clean`.
4. By default `sieve run` runs the project's `Test` tasks, not linters; `sieve run -- check` runs
   `check`.
5. Tests I request explicitly (`--tests …`) always run.
6. When nothing changed since the last passing run, no build starts.
7. Plain `./gradlew` invocations behave exactly as before.

As a Sieve maintainer:

8. Building Sieve with the agent needs a JDK 17+ `javac`, not 24+.
9. Planted bugs and a commit walk run on Gradle projects, and any missed failure exits with 1.
10. The Maven local mode keeps its behavior and passes its existing tests on the new agent.

## Assumptions and limits

The assumptions of PRD 00 stay: observed paths cover a test's behavior, and tests are independent.
A JVM whose class files the vendored ASM cannot read keeps the agent inactive, so every test runs.

## Issues

| # | Slice | Type | Blocked by |
|---|---|---|---|
| 16 | [Agent on Java 17+ with vendored ASM](16-agent-on-java-17-with-asm.md) | AFK | — |
| 17 | [Gradle: the agent writes test records](17-gradle-agent-writes-records.md) | AFK | — |
| 18 | [Gradle: skip tests whose records are unchanged](18-gradle-skip-unchanged.md) | AFK | 17 |
| 19 | [Gradle developer loop](19-gradle-developer-loop.md) | AFK | 18 |
| 20 | [Catalog, planted bugs, and commit walk on Gradle](20-gradle-catalog-and-walk.md) | AFK | 18 |
| 21 | [Measure on the Gradle services](21-measure-gradle-services.md) | HITL | 16, 19, 20 |
| 22 | [Library jar evidence for dependency bumps](22-library-jar-evidence.md) | AFK | 18 |
