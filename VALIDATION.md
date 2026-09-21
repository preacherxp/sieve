# Validation record

Validated on 2026-09-21 with OpenJDK/Temurin 17.0.12, Maven 3.9.9,
Gradle 8.12.1, and Python 3.

## Checks

- Both equivalent baselines: 12 test classes, 13 invocations, no failures.
- 17 mutations per build tool: exact expected failing classes, complete test inventory.
- New-test mutation: 13 classes / 14 invocations.
- Six harness checks: mutation application for both tools, source parity, rejection of
  missing tests, conservative versus exact selection, invalid selection payloads,
  parameterized/skipped/failing XML report parsing.
- Exact selection example checked through the public harness CLI.

`validation-summary.json` records the final result of each of the 36 build executions.
A successful mutation validation means its deliberately introduced failures matched
expectations, including the cases where no failures were expected.

## Environment detail

Maven resolved dependencies from Maven Central. Gradle builds used a temporary external
init script pointing at the same downloaded Maven artifacts as a local file repository,
with offline dependency resolution, because Java proxy behavior differed in this runner.
The delivered Gradle project retains normal `mavenCentral()` configuration. No validation
machine paths or proxy settings are required by the projects or included in their builds.

## Limits

These results validate the fixture applications and oracle. No test selection engine is
implemented in this bundle, so engine recall, runtime overhead, cache correctness, and
filter application still need to be measured against your implementation. The included
GitHub Actions workflow has been provided for future runs, not executed on GitHub here.
