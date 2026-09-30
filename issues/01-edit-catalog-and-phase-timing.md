# 01 Edit catalog with phase timing

Type: AFK

Status (2026-09-30): implemented as `sieve catalog` and `sieve classify` (`src/catalog.rs`,
`src/timing.rs`), with example catalogs for `samples/bookstore` and `projects/records`.
Phases come from timestamped build output and add up exactly; Spring Boot and Testcontainers
startup are reported alongside (issue 06). Runs in a temporary copy by default.

## Parent PRD

[00](00-prd-local-first-test-records.md)

## What to build

A way to measure feedback time per kind of edit on a real project, with a phase breakdown, so every
later slice can show what it saves.

A catalog file lists typical edits: a method body, a listener body, a JSON fixture, test
configuration, a new field, a new test, a rename, a docs-only change, a POM edit. Each edit is a
search-and-replace on a workspace file, tagged with its edit kind. Sieve applies each edit to HEAD,
runs `sieve run`, reverts it, and repeats for a configurable number of samples. It reports median and
range per edit kind next to a native full build and `sieve run --full` on the same warm tree.

A separate history classification assigns each recent commit's changed paths to edit kinds without
building anything, and produces the weights for a weighted total.

The phase breakdown splits every run into Maven start, compilation, test JVM start, and test
execution; context and container startup join once issue 06 lands. Before the agent exists, phases
come from timestamped Maven output and Surefire/Failsafe reports.

## Acceptance criteria

- [ ] A catalog runs end-to-end on `samples/bookstore` with a committed example catalog. Results
      include raw samples, median and range per edit kind, and the weighted total.
- [ ] History classification produces edit-kind weights for the last N first-parent commits without
      starting a build.
- [ ] Each run's phases add up to its wall-clock time within a stated tolerance. Unit tests cover log
      and report parsing.
- [ ] Results are written as JSON plus a readable summary under the `--output` directory. Catalogs for
      private projects can live outside the repository.
- [ ] README documents the catalog format and command.

## Blocked by

None - can start immediately.

## User stories addressed

- User story 18
