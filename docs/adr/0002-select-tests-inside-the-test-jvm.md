---
status: accepted
---

# Select tests inside the test JVM

Sieve used to drive the build: compile, select in Rust, then start Maven again with an excludes file, which cost a second build-tool start on every class-level run and made commits that need nearly every test slower than the full suite. Selection now happens at JUnit Platform test discovery: a Sieve jar on the test classpath registers a discovery filter that asks the `sieve` binary which test classes to run, and the same jar writes test records. The build is the developer's normal command with one build-tool start, `sieve run` becomes a thin wrapper that adds the opt-in flag, the agent, and the base, and the selection rules stay in Rust.

## Considered Options

- Maven core extension hooking between `test-compile` and `test`: also one Maven start, but Maven-only and tied to Maven internals.
- Keep orchestrating two builds: no JVM-side code, but it pays the second start and plain `mvn` or IDE runs get no selection.

## Consequences

- Tests Sieve drops are absent from build reports rather than reported as skipped.
- Only JUnit Platform tests can be filtered.
- The jar is embedded in the `sieve` binary and reaches the test JVM as a `-javaagent` through the build's `argLine`, so service POMs stay untouched and jar and binary versions cannot drift apart. Publishing the jar to a registry was rejected because it needs a release pipeline and allows version skew.

## Amendment: the agent reaches the test JVM through `JDK_JAVA_OPTIONS`

Passing the agent through `-DargLine` fails on POMs that declare an `argLine` property (the usual JaCoCo setup): Surefire's late `@{argLine}` replacement reads the project property and ignores the user property. `sieve run` therefore sets `JDK_JAVA_OPTIONS`, which every `java` launch reads regardless of the POM, and the agent stays inactive in the build tool's own JVM. `sieve env` prints the same value for plain Maven runs.
