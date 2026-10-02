# Follow-up work after the local-first effort

Issues 00–14 are closed. This list collects what they left open, grouped by kind. Results and
catalogs for internal services stay outside this repository.

## Verification

- [ ] Re-run the planted-bug catalogs and commit walks done before 2026-10-01: their planted runs
      used a different invocation than the normal runs, selected every test, and so could not
      miss a failure (fixed in the runner; see issue 20).

- [ ] Run the new CI job ("Test records (Maven, JDK 25)") on GitHub, and the JDK 25 setup in the
      jobs that compile Rust.
- [ ] Walk the commons-text window from `docs/performance.md` with `sieve replay --walk --plant`
      (issue 12): the walk has only run on local histories and internal services so far.
- [ ] Exercise `mvnd` (`--with mvnd`) and check that the daemon passes `JDK_JAVA_OPTIONS` to test
      JVMs; until then the lever stays opt-in (issue 10).
- [ ] Run the native record tests once with each lever switched on and off (issue 10).

## Oracle coverage

- [ ] `fixtures verify --records`: prime records, apply a scenario, and check with the independent
      oracle, plus a records scenario in `scenarios.json` (issue 04).
- [x] Scenarios for a changed method annotation, a removed method, and a supertype change
      (issue 05), a static-field-only dependency, an edited meta-annotation, a `@ContextHierarchy`
      parent, and `@TestPropertySource` files (from the code review). Done as native tests in
      `tests/records.rs`.
- [x] Run the context-test scenarios in both test-class orders (issue 06).

## Precision

- [x] Weight edit kinds without automated dependency updates: optionally leave out commits that
      touch only the POM when classifying history. Done: the catalog reports both totals; only
      5–10 of 34–50 commits per benchmark service were POM-only.
- [x] Narrow configuration edits: done statically (ADR 0004). A changed key reruns the users of
      project classes that name it; keys only the framework reads still rerun every reader.
- [ ] Narrow framework-read configuration keys (`spring.*`, `server.*`) by the auto-configuration
      that binds them, and migrations that alter existing tables by the tables tests touch.
- [ ] Member-level shapes for components: an annotation added to one method reruns all users of
      the class, not only callers of that method.
- [ ] Lazily created beans count only for the test class that first used them; attribute them to
      the context that holds them.
- [ ] Revisit the AOT class cache once the JDK can cache classes with directories on the class path
      (issue 10).

## Service-side (outside this repository)

- [ ] Apply the test-setup audit per benchmark service: containers once per JVM with
      `withReuse(true)`, no inherited `@DirtiesContext`, shared `@MockitoBean` sets, unique test data,
      and removal of long fixed waits (issue 13). The audits are kept with the private results.
- [ ] Re-run the catalogs and a commit walk with planted bugs on each service with the current
      binary, and decide the speed-up defaults from them (issue 14).
- [ ] Decide what, if anything, to publish in `docs/performance.md` about the internal services;
      internal names and numbers stay private.

## Housekeeping

- [ ] Optionally regroup the local-first commits: the first commit also holds an early version of
      the agent, because files were already staged when the commits were queued.
