# 03 Agent writes test records

Closed 2026-09-30: implemented; the CI job still needs a GitHub run ([TODO.md](TODO.md)).

Type: AFK

Status (2026-09-30): implemented. `agent/` (probe, agent, API stubs), `build.rs`, the `agent`
Cargo feature, `src/records.rs`, and `projects/records`. The fixture compiles for Java 21
(Spring 6.2 scanning) and runs on a Java 25 test JVM. The agent reaches the test JVM through
`JDK_JAVA_OPTIONS`, not `argLine`: see the ADR 0002 amendment. The new CI job is untested on
GitHub.

## Parent PRD

[00](00-prd-local-first-test-records.md)

## What to build

The thinnest end-to-end path for recording. A single-module Maven project opts in (proposed:
`"records": true` in `impact.json`), `sieve run` starts Maven once with the embedded agent, and every
test class leaves a test record in `.sieve/`. No test is skipped yet.

- `build.rs` compiles the agent's Java sources (Java 24+, `java.lang.classfile`, no third-party
  dependencies) into a jar that the binary embeds. A missing or too-old `javac` fails the build with a
  hint, and a Cargo feature builds Sieve without the agent (static selection only).
- `sieve run` extracts the jar to a user cache directory, verifies its hash on every use, and passes
  it through `-DargLine=-javaagent:<jar>=<options>`. When the POM's Surefire or Failsafe `argLine`
  lacks `@{argLine}`, Sieve prints the one-line POM change instead of guessing.
- The agent adds method-entry probes only to classes from the project's own output directories. A
  JUnit Platform listener, registered through ServiceLoader, collects the methods each test class
  executed and whether it passed. At the end of the run the `sieve` binary finalizes the records and
  adds bytecode hashes, so hashing has a single implementation, in Rust.
- Records accumulate across passing runs. A failing run marks the record failed without adding to it.
- `.sieve/` carries its own `.gitignore` (`*`) and survives `mvn clean`.
- A new fixture, `projects/records` (Java 25, single-module Maven, plain JUnit tests), demonstrates
  the records.

## Acceptance criteria

- [ ] Two runs of `projects/records` produce records listing exactly the project methods each test
      class executed, with hashes, JDK version, and outcome.
- [ ] `sieve run` starts Maven exactly once in this mode, checked with a fake executable that counts
      invocations.
- [ ] A tampered extracted jar is detected and replaced before use.
- [ ] Building without the agent feature still passes the existing test suite.
- [ ] CI builds the agent with JDK 25 and runs the new native tests; existing jobs are unaffected.
- [ ] Projects without the opt-in behave exactly as before.

## Blocked by

None - can start immediately.

## User stories addressed

- User stories 12, 13, 22, 23
