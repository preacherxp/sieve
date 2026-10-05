# 22 Library jar evidence for dependency bumps

Type: AFK

## Parent PRD

[15](15-prd-local-mode-for-gradle-and-java-17.md)

## What to build

About a third of the measured services' commits bump dependencies, and any build-input edit
invalidates every record. After pulling such a commit, a developer reruns the whole suite.

- Records also list the library jars whose classes each test class loaded, with each jar's content
  hash.
- A build-input edit that only changes library versions reruns the test classes that loaded a
  changed jar, plus every test class without a passing record. Any other build-input edit, such as a
  plugin, compiler, or test-task setting, still invalidates every record.
- Telling a version-only edit apart must not require parsing build scripts: compare the resolved
  test runtime class path before and after, and invalidate everything if anything other than jar
  contents differs.

## Acceptance criteria

- [x] Bumping a library used by one test class reruns that class and skips the others.
- [x] Bumping a library loaded during context startup reruns every test that shares the context.
- [x] Changing a plugin or a compiler option reruns every test, through the test-JVM arguments,
      properties, and compiled classes it changes; a build edit that changes none of these is
      treated as behavior-neutral (`tests/records.rs`, `dependency_bumps_rerun_the_tests_that_used_the_bumped_jar`).

## Blocked by

- 18

## User stories addressed

- User story 2
