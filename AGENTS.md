# Agent instructions: GitHub Actions integration

Apply these instructions when adding or changing Sieve integration in an existing
repository's GitHub Actions CI. Keep changes focused on the requested integration.

## Current capabilities

- Sieve is a Rust CLI named `sieve`. It currently supports Java and
  Kotlin/JVM projects using Maven or Gradle, with conservative module selection.
  Projects can opt into bytecode-based class-level selection
  (`"class_level": true`); it compiles the selected modules before running tests.
- Local mode (`"records": true`, single-module Maven or Gradle, Java 17+ test JVM) is for developer
  machines, not CI: it keeps per-test records in `.sieve/` and loads an embedded agent
  through `JDK_JAVA_OPTIONS` (Maven) or Sieve's init script (Gradle). Do not enable it in
  consumer CI workflows.
- Language independence is a future direction, not an implemented capability.
  Do not advertise support for other languages or invent adapter commands.
- Read `README.md`, `Cargo.toml`, and `src/gradle.init.gradle` for current setup
  requirements and supported layouts. Do not assume arbitrary monorepos work.
- `.github/workflows/java-tests.yml` demonstrates this repository's fixture CI.
  It is not a general-purpose workflow for consumer repositories.

## Integrate with the existing job

- Inspect the workflow, build wrappers, test commands, profiles, matrices, caches,
  and report paths before editing. Preserve unrelated jobs and local changes.
- Keep the existing JDK setup, dependency caches, permissions, and report uploads.
  Replace only the test execution step and add the setup needed by Sieve.
- Avoid introducing a custom action, reusable workflow, or new dependency when
  ordinary workflow steps suffice. Do not upgrade unrelated action versions.
- For a consumer repository, install the release binary with `install.sh` fetched from
  a verified release tag and `SIEVE_VERSION` pinned to the same tag; it checks the
  release checksum. Where no prebuilt binary exists, install from the Git repository
  using `--locked` and `--rev` pinned to a verified commit, with a Rust toolchain
  compatible with `Cargo.toml` and a JDK 17+ `javac`. Never invent a release tag or
  commit SHA.
- In Sieve's own CI, build the checked-out source instead of installing a remote
  version, so the workflow exercises the changes under review.
- Run `sieve init` once during project setup using the CI build
  environment. Review and commit `impact.json` and any generated Maven POM edits.
  Do not run `init` on every CI execution; it refuses existing configuration.
- Preserve declared dependency edges, including runtime and resource dependencies.
  Update the configuration when the project graph changes.

## Select the comparison base safely

- Set `fetch-depth: 0` on the existing checkout step. Use
  `persist-credentials: false` when later steps do not need Git credentials.
- For ordinary `pull_request` jobs, retain the default merge checkout and pass
  `github.event.pull_request.base.sha` as `--base`.
- Default other events to `--full`. If the existing workflow intentionally selects
  tests on pushes, use `github.event.before`, falling back to `--full` when it is
  empty or the all-zero SHA. Keep explicit full-suite jobs as full-suite jobs.
- Missing Git history must retain Sieve's full-suite fallback. Invalid setup or
  build failures must still fail the job.
- Pass event values through environment variables and quote shell expansions.
  Use Bash arrays for optional arguments, not interpolated shell command strings.
- Execute pull-request code under `pull_request`, not `pull_request_target`.

## Execute and report

- Use `sieve run --workspace PATH` with either `--base REV` or `--full`.
  Prefer the project's existing wrapper; use `--executable` only when needed.
- Forward applicable build flags after `--`. Check command equivalence: Sieve
  currently runs Maven `clean verify` or Gradle `clean check`, limited for a partial
  selection to `-pl <modules> -am` or `:<module>:check` tasks; it does not preserve
  arbitrary existing tasks automatically. Aggregating steps (coverage reports, packaging
  of unselected modules) see only the built modules. `NONE` still runs `clean`.
- Use `sieve replay --workspace PATH --commits N --run` on the consumer repository
  to measure selection modes, time saved, and missed failures before relying on it.
- Place execution before steps that consume build outputs, and account for
  existing coverage, packaging, or custom test tasks before replacing commands.
- Write selection JSON under `$RUNNER_TEMP` or an ignored directory so reports
  cannot become untracked inputs that force full selection.
- Preserve test failure propagation. Do not add `continue-on-error`, `|| true`,
  or fixture-only failure-ignore flags to production test execution.
- Publish available selection JSON and native test reports even when tests fail;
  guard optional files and use `if: always()` for reporting steps.
- Retain existing full-suite validation during rollout. Do not remove required
  checks or change branch-protection settings as part of routine integration.

## Verify the change

- Run `actionlint` for workflow edits when available; report if it is unavailable.
- Check PR-base selection, full-run events, missing-history fallback, and failure
  propagation using existing checks or a focused runnable check when needed.
- For CLI or adapter changes, run the relevant Rust and build integration checks
  documented in `README.md`. Keep fixture expectations independent of selection
  logic; do not weaken the oracle to make a changed selector pass.
- Report which checks actually ran and any remaining environment limitations.
  Do not claim a GitHub Actions run succeeded based solely on local validation.
