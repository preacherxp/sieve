---
status: proposed
---

# Local mode for Gradle and Java 17 test JVMs

Static selection cannot make single-module projects faster. On a single-module Spring Boot service built with Gradle, measured on 2026-09-30, module selection runs the whole suite for every source edit. Class-level selection was never faster than the full build in nine replayed commits (1,048 s full against 1,075 s selected): every component edit reached every context test, and the second Gradle start cost more than the tests it skipped. Local mode already solves both problems (ADR 0001, ADR 0002), but only for Maven and Java 24+ test JVMs, and most services build with Gradle or test on Java 17 or 21. Local mode therefore becomes the path for single-module projects on both build tools. The agent instruments classes with a vendored, relocated copy of ASM instead of the Class-File API, so it runs on Java 17+ test JVMs. On Gradle, an init script attaches the agent to every `Test` task. The scope stays the developer loop: CI keeps static selection and full-suite validation.

## Considered Options

- Keep the Class-File API (Java 24+): no third-party code, but services testing on Java 17 or 21 gain nothing until they upgrade their test JVM.
- Class-File API on 24+ and ASM below: no lag behind new JDKs, but two transformers to write, test, and keep equivalent.
- ASM as a test dependency of the project: no vendoring, but it edits build files and can clash with the project's own ASM version.
- Attach the Gradle agent through `JDK_JAVA_OPTIONS`, as on Maven: no init-script code, but every `java` launch reads it, including the long-lived Gradle daemon and the Kotlin compile daemon. In the 2026-09-30 measurements a similar option (`JAVA_TOOL_OPTIONS`) broke the Kotlin daemon handshake, and Kotlin fell back to slower compilation.

## Decision details

- ASM sources (BSD-3-Clause) are vendored under `agent/` at a pinned version. `build.rs` compiles them into the package `sieve.agent.asm`, so they cannot clash with an ASM on the test class path. The license file ships with the jar. The agent compiles with `--release 17`, so building Sieve needs a JDK 17+ `javac` instead of 24+.
- The Gradle init script adds the agent through a `jvmArgumentProviders` entry on each `Test` task, only when `sieve run` enables records. Plain Gradle invocations are unaffected.

## Consequences

- One transformer implementation, at the cost of vendored third-party code that must be upgraded for every new class-file version. A test JVM whose class files ASM cannot read keeps the agent inactive, and every test runs.
- Gradle local mode keeps the daemon and Gradle's own incremental compilation: Gradle removes the outputs of deleted sources itself, so Sieve never adds `clean` there.
- Gradle may restore a `Test` task from its up-to-date check or build cache, which writes no records. That is safe only because identical task inputs mean identical classes and resources; issue 18 must confirm it before relying on it.
- Records stay local. Producing or sharing them in CI is not part of this decision.
