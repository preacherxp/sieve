# Separate test suites (Gradle)

A Java 17 example with two modules: `engine` supplies a formatter and shared
`java-test-fixtures`; `app` has parameterized unit tests and a separate
`src/integrationTest` source set with its own resource and `Test` task. `check`
runs both suites: three classes, four invocations. No external services are needed.

```bash
cd samples/gradle-test-suites
gradle clean check
sieve run --workspace . --full
# In a standalone Git copy, commit the baseline, edit a source, then:
sieve run --workspace . --base HEAD
```

Use Gradle 7.6.3+ on a compatible JDK. `impact.json` was generated with `sieve init`
and enables class-level selection. From the repository root:

```bash
IMPACT_TOOL=gradle cargo test --locked --test native native_sample_mutations -- --ignored --nocapture
```

The independent oracle checks native full builds and both selection modes for a
formatter bug, shared fixture bug, integration resource change, integration test
edit, unrelated unit test failure, build-file edit, and docs-only edit. It checks
exact classes, invocation counts, failures, and nonzero failure status.
`--continue` lets independent Gradle test tasks finish while preserving failure;
no test failures are ignored. `IMPACT_GRADLE` overrides `gradle`. The existing CI
native checks include it.
