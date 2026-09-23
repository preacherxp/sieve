# Validation record for the original fixtures

Validated locally on 2026-09-21 with Rust 1.92.0, Java 17.0.20.1,
Maven 3.9.9, and Gradle 8.13. GitHub Actions pins Gradle 8.12.1.

## Checks

- Rust formatting, Clippy, and unit/integration checks.
- All 18 scenario selections for both build tools, checked against the independent
  manifest. Checks include committed/staged/untracked changes, renames, merge-base
  comparison, missing history, malformed selections, and build-failure propagation.
- Equivalent Java/Kotlin baselines: 13 test classes / 14 invocations, no failures.
- All 18 mutations plus baseline, both full and selective execution, on Maven and
  Gradle: **76 passing benchmark runs**. A mutation passes when its expected failing
  classes are observed, not when the deliberately broken application passes tests.
- Exact executed classes, invocation counts, skipped tests, and failure sets checked
  from native XML reports. New-test full runs have 14 classes / 15 invocations.
- Installation tested on Maven and Gradle multi-module projects with the old adapter
  removed, including actual selective execution and refusal to overwrite setup.
- Standalone Kotlin DSL project installation, Kotlin compilation, and failing test
  execution verified. No build-file adapter edits are needed for Gradle.
- GitHub workflow syntax checked with actionlint.

`validation-summary.json` contains the 76 full/selective results. Detailed build
logs remain under the ignored `validation-results/` directory.

## Limits

The selector intentionally works at module granularity. A pricing change selects
8 classes / 9 invocations, including the Kotlin test; a runtime change selects its
5 integration classes. Unknown changes fall back to ALL. The benchmark establishes
behavior for these fixtures; it is not a proof of soundness for arbitrary JVM builds.

The local tests used Gradle 8.13; the checked-in CI configuration tests 8.12.1.
Installation tests require a JDK and native build tools and are explicitly marked
ignored in the fast Rust test run; the scheduled fixture workflow runs them.

## Expanded fixture checks (2026-09-21)

The current fixtures have 20 mutation scenarios and 15 baseline test classes with
17 invocations. Rust formatting, Clippy, and unit/integration checks passed. All
20 mutations plus the baseline passed selective verification on Maven 4.0.0-rc-6
and Gradle 8.13: 42 benchmark runs. The Kafka and Redis Testcontainers tests compile
with Maven 4; Docker is unavailable locally, so their execution awaits CI. The older
76-run summary above belongs to the original 18-scenario fixture set.

## Coverage implementation (2026-09-21)

Validated with Rust 1.92.0, Java 17.0.20.1, Maven 4.0.0-rc-6, and Gradle 8.13.

- `cargo fmt --check`, Clippy with warnings denied, and actionlint passed.
- Fast Rust checks: **27 passed**; six JVM/Docker integration checks are explicitly
  ignored by the default Rust run.
- Five ignored integration checks were run explicitly and passed: Maven/Gradle
  installation, standalone Kotlin DSL installation, deep native dependency graphs,
  Java consumers of Kotlin providers, and unsupported Gradle layout/plugin rejection.
- Native graph checks cover runtime reflection, resources, test JARs/test fixtures,
  preserved default Maven profiles, custom Gradle Test tasks, root tests, graph
  refresh, clean report isolation, assertion/compilation/resolution failures,
  invalid compile cycles, and missing JDK failure propagation.
- **88 fixture runs passed**: baseline plus all 21 mutations, full and selected,
  on both Maven and Gradle. The independently deleted test yields 16 full
  invocations and four selected invocations. Logs and JSON are retained under
  `validation-results/coverage-2026-09-21/`.
- **108 exploratory timing runs** passed the independent execution oracle.
  [Performance results](docs/performance.md) record raw samples and limitations;
  overlapping Rust work makes small timing differences inconclusive.
- Docker is unavailable locally. The new Kafka/Redis selection, avoided-startup,
  image-pull/startup-timeout, and cleanup checks are wired into the existing CI
  jobs, but their execution still needs CI confirmation. Existing native smoke
  tests remain scheduled even if the new selection check fails.
- This change has not been run on GitHub Actions. The recorded GitHub job costs
  refer to the preceding committed workflow, not these local changes.

## Class-level selection (2026-09-23)

Validated with Rust 1.92.0, Java 17.0.20.1, Maven 3.9.9, and Gradle 9.6.1.

- `cargo fmt --check`, Clippy with warnings denied, and fast Rust checks passed.
- Ignored native checks passed on Maven and Gradle: the new single-module class-level
  run (direct and transitive callers, a Failsafe test, `Class.forName` use, no reaching
  test, constant fallback, failure and compile-error propagation), the native graph
  check, the Kotlin provider check, and unsupported Gradle layout rejection.
- Surefire 3.5.2 was checked by hand: an excludes file removes listed classes and drops
  the default `**/*$*` exclude, which Sieve therefore repeats.
- `samples/bookstore/demo.sh` results are recorded in its README. Not yet run on GitHub
  Actions.
