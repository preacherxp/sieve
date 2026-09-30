# 10 Local speed-ups

Closed 2026-09-30: `reuse` and `jgitver` are on, `mvnd` is opt-in, and the AOT cache is dropped; `mvnd` checks moved to [TODO.md](TODO.md).

Type: AFK

Status (2026-09-30): `reuse` and `jgitver` are on by default and `mvnd` is opt-in (its daemon
may not pass the agent option to test JVMs; untested here, no mvnd installed). The AOT cache is
dropped: JDK 25 cannot create one while the class path holds a non-empty directory, and
Surefire's always holds `target/classes`. Container reuse is checked by container IDs in
`tests/records.rs` (`projects/records-reuse`). `catalog --levers` reports each lever.

## Parent PRD

[00](00-prd-local-first-test-records.md)

## What to build

Cut the fixed cost that remains per run once only a few tests execute. The wrapper switches each lever
on in local mode, each can be switched off on its own, and the edit catalog measures each one. Issue 14
decides the defaults.

- Testcontainers reuse (`TESTCONTAINERS_REUSE_ENABLE=true`), effective for containers that opt in with
  `withReuse(true)`.
- A JDK 25 AOT class cache for the test JVM, kept under `.sieve/` and rebuilt when dependencies change.
  It needs `-Dsurefire.useManifestOnlyJar=false` so that the classpath stays stable between runs.
- `mvnd` when it is installed.
- Skipping the jgitver extension's version calculation (`-Djgitver.skip=true`) when a project uses it
  and no test depends on the project version.

## Acceptance criteria

- [ ] Each lever has an on/off switch and appears in `--output`.
- [ ] Container reuse is confirmed by container IDs surviving between two runs.
- [ ] The catalog reports the saving per lever on a public sample.
- [ ] No lever changes which tests run or their results: the oracle scenarios pass with each lever on.

## Blocked by

- 01
- 09

## User stories addressed

- User story 17
