# Runtime plugins (Maven)

A three-module Java 17 example: `api` defines a service, `provider` implements it,
and `app` discovers it with `ServiceLoader`. The provider is a **runtime** dependency;
the application never imports its implementation. A properties file supplies the
greeting. Two Failsafe classes run three test invocations, including an independent
status check and a parameterized service test. No external services are needed.

```bash
cd samples/runtime-plugins
mvn clean verify
sieve run --workspace . --full
# In a standalone Git copy, commit the baseline, edit a source, then:
sieve run --workspace . --base HEAD
```

`impact.json` was generated with `sieve init` and enables class-level selection.
Keep the runtime edge when refreshing the graph. From the repository root, run
the mutation checks on temporary copies:

```bash
IMPACT_TOOL=maven cargo test --locked --test native native_sample_mutations -- --ignored --nocapture
```

The independent oracle checks native full builds and both selection modes for a
provider bug, changed greeting resource, deleted service registration, changed
test assertion, unrelated status bug, build-file edit, and docs-only edit. It
requires exact classes, invocation counts, failures, errors, and nonzero failure
status. `IMPACT_MAVEN` overrides `mvn`. The existing CI native checks include it.
