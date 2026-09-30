# PRD: Local-first test selection with test records

Status: agreed in the design session of 2026-09-30. Decisions:
[ADR 0001](../docs/adr/0001-record-runtime-evidence-for-context-tests.md),
[ADR 0002](../docs/adr/0002-select-tests-inside-the-test-jvm.md). Terms: [CONTEXT.md](../CONTEXT.md).

## Problem

On single-module Spring Boot services whose expensive tests use Testcontainers (Kafka, MongoDB),
Sieve saves little time:

- An edit to any Spring component selects every context test. Framework dispatch (Kafka listeners,
  HTTP handlers) leaves no bytecode edge from a test to the code it runs, so static analysis cannot
  rule any of them out, and they are the expensive tests.
- Class-level runs start Maven twice, and edits to `generated` inputs also generate at the base, so
  commits that need nearly every test end up slower than the full run.
- Every run starts cold with `clean`: recompilation, source generation, and Boot repackaging, even on
  a developer machine whose build output is still valid.
- The services start containers per test class and rebuild Spring contexts, which inflates full and
  selected runs alike.

## Goal

Minimal feedback time for developers running tests locally.

- Zero missed failures: the fixture oracle, planted bugs, and one commit walk must all pass.
- No regression by construction: a selected run never does more build work than a full run (at most
  one Maven start, no second lifecycle pass).
- Objective: the lowest total feedback time over each benchmark service's edit catalog, weighted by
  how often each edit kind occurs in its history.

The benchmark is four internal single-module services. Their identities and private results stay
outside this repository.

## Solution

- Every run keeps a test record per test class. A test is skipped when its last run passed and none
  of its recorded methods or resources changed, or when static analysis shows it reaches no change
  since a green base (ADR 0001).
- Selection happens inside the test JVM. A Sieve jar, injected as `-javaagent` through the build's
  `argLine`, registers a JUnit Platform discovery filter and writes test records. The `sieve` binary
  makes every decision, and `sieve run` becomes a thin wrapper (ADR 0002).
- The agent needs Java 24+ and uses the JDK's Class-File API. `build.rs` compiles it, and the binary
  embeds it.
- `sieve run` builds incrementally, cleans only when stale output is possible, skips Boot packaging,
  and starts no build when nothing changed.
- Optional local speed-ups (Testcontainers reuse, an AOT class cache, `mvnd`, skipping jgitver) are
  measured and individually switchable.

## Scope

In: single-module Maven projects, JUnit Platform tests, test JVMs on Java 24+, developer machines.

Later: CI integration and records produced in CI, Gradle, multi-module projects. Projects on older
JDKs and all layouts outside this scope keep today's module-level and class-level behavior unchanged.

## User stories

As a developer on a single-module Maven service:

1. When I rerun tests without edits, no build starts and I get the verdict immediately.
2. When I edit one method body, only the tests that executed it run, including context tests.
3. When I edit a JSON fixture or test configuration, only the tests that read it run.
4. `sieve run` never does more build work than the full run it replaces.
5. Plain `mvn test` or `mvn verify` gets the same selection when I opt in through `JDK_JAVA_OPTIONS`.
6. Tests I request explicitly (`-Dtest=…`, a single test from the IDE) always run.
7. A structural change runs every test whose record touches the changed class or its project
   supertypes and subtypes.
8. An edit to Spring wiring runs every context test whose kind loads the change.
9. Work done while a context starts counts for every test that shares that context.
10. A test that failed runs again until it passes.
11. A test without a passing record falls back to static selection against `--base` when given, and
    runs otherwise.
12. A test's record accumulates across its passing runs until the test class itself changes.
13. Records survive `mvn clean` and are never tracked by Git.
14. Switching JDK invalidates records, and a run that executes test classes in parallel inside one
    JVM keeps no records.
15. Builds are incremental: `clean` runs only when stale output is possible, and Boot `repackage` and
    `build-info` are skipped.
16. `--output` gives a reason for every selected test.
17. Local speed-ups are switched on by the wrapper, and each can be switched off.

As a Sieve maintainer:

18. The edit catalog measures feedback time per edit kind with a phase breakdown, weighted by the
    project's history.
19. Planted bugs prove that the selected run fails whenever the full run fails.
20. A commit walk replays recent history in one working tree, carrying records and build output
    forward.
21. The fixture oracle has a scenario for every new selection rule.
22. The agent is built from source by `build.rs`, embedded in the binary, and verified by hash when
    extracted; a build switch produces a binary without it.
23. Adopters on older JDKs, Gradle, or multi-module layouts see no change in behavior.

## Assumptions and limits

- Every path a test can take has shown up in one of its passing runs since the test class last
  changed, and tests do not depend on side effects left by other tests (see issue 13).
- Not tracked: environment variables, system properties, external service behavior, floating Docker
  image tags.

## Issues

| # | Slice | Type | Blocked by |
|---|---|---|---|
| 01 | [Edit catalog with phase timing](01-edit-catalog-and-phase-timing.md) | AFK | — |
| 02 | [Baseline the benchmark services](02-baseline-benchmark-services.md) | HITL | 01 |
| 03 | [Agent writes test records](03-agent-writes-test-records.md) | AFK | — |
| 04 | [Skip tests whose records are unchanged](04-skip-tests-with-unchanged-records.md) | AFK | 03 |
| 05 | [Structural changes invalidate records per class](05-structural-changes.md) | AFK | 04 |
| 06 | [Context startup and Spring wiring](06-context-startup-and-spring-wiring.md) | AFK | 04 |
| 07 | [Record classpath resource reads](07-resource-reads.md) | AFK | 04 |
| 08 | [Static fallback for tests without a record](08-static-fallback.md) | AFK | 04 |
| 09 | [Incremental local builds and the no-change fast path](09-incremental-builds-and-fast-path.md) | AFK | 04 |
| 10 | [Local speed-ups](10-local-speed-ups.md) | AFK | 01, 09 |
| 11 | [Planted bugs](11-planted-bugs.md) | AFK | 01, 04 |
| 12 | [Commit walk](12-commit-walk.md) | AFK | 09, 11 |
| 13 | [Service-side test setup fixes](13-service-side-test-setup-fixes.md) | HITL | — |
| 14 | [Final evidence and defaults](14-final-evidence-and-defaults.md) | HITL | 02, 05–13 |
