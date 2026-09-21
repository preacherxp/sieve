# Validation record

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
