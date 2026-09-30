# 04 Skip tests whose records are unchanged

Closed 2026-09-30: implemented; `fixtures verify --records` moved to [TODO.md](TODO.md).

Type: AFK

Status (2026-09-30): implemented, except `fixtures verify --records`. The scenarios run as
native tests in `tests/records.rs` against `projects/records` instead. `sieve env` prints a
`JDK_JAVA_OPTIONS` value (ADR 0002 amendment). Context tests became eligible together with
issue 06.

## Parent PRD

[00](00-prd-local-first-test-records.md)

## What to build

The first time saving. At test discovery, the agent's JUnit Platform discovery filter asks the `sieve`
binary which test classes must run and drops the rest. Everything uncertain stays conservative, so
the slice is safe on its own.

- Rust computes per-method bytecode hashes that ignore debug attributes (line numbers, local variable
  tables) and resolve constant-pool references to their values. Comment-only edits and constant-pool
  reordering therefore leave hashes unchanged.
- A test class is skipped only when its last run passed, its JDK matches, the test class itself is
  unchanged, and every recorded method's hash is unchanged. Everything else runs: a failed or missing
  record, any structural change in the project, any resource or build-input edit, and every context
  test until issue 06 lands.
- A run that executes test classes in parallel inside one JVM keeps no records and skips nothing.
- Tests requested explicitly (`-Dtest=…`) are never dropped. Runs without Sieve's opt-in, such as IDE
  runs, are unaffected.
- `--output` lists every test class with its reason: changed method, failed last time, no record, JDK
  changed, or conservative fallback.
- `sieve env` prints the `JDK_JAVA_OPTIONS` value that gives plain `mvn test` or `mvn verify` the same
  selection.
- `fixtures verify` gains a records mode: prime records with an unmodified run, apply a scenario, run
  the selected tests, and check required tests and expected failures with the independent oracle. New
  scenario: a body change in a method that only one unit test executes.

## Acceptance criteria

- [ ] On `projects/records`, a rerun without edits executes no tests, and a one-method body edit
      executes only the test classes whose records contain that method.
- [ ] A comment-only edit selects nothing. An edit that reorders the constant pool selects only the
      tests of the methods that actually changed.
- [ ] A failed test runs again on every run until it passes.
- [ ] Explicit `-Dtest=` runs execute the requested tests even when their records are valid.
- [ ] `fixtures verify --records` passes the new scenario and every applicable existing scenario with
      zero missed failures.
- [ ] Maven still starts exactly once per `sieve run`.
- [ ] README documents local mode, the opt-in, and `sieve env`.

## Blocked by

- 03

## User stories addressed

- User stories 2 (tests that start no context), 4, 5, 6, 10, 14, 16, 21
