# Apache Commons Text TEXT-158

This case reproduces the real [TEXT-158 upstream fix](https://github.com/apache/commons-text/commit/38727e31d978350b76edba1683a081a114fe345f): two empty strings have Jaccard similarity `1` and distance `0`.
The buggy parent revision returns the opposite values. Both upstream Jaccard regression test classes detect the error; an unchanged upstream Hamming-distance test class provides an unrelated control.

All Java source and test files are copied **byte for byte** from Apache Commons Text. Their original Apache headers, `LICENSE.txt`, and `NOTICE.txt` are retained. `provenance.json` records immutable revisions, original paths, SHA-256 checksums and Git blob IDs. The POM, this document, and Sieve's Rust harness are the only Sieve-authored parts.

- Fixed sources and all tests: `38727e31d978350b76edba1683a081a114fe345f`.
- Buggy production file under `regression/`: parent `ac09fd74ed1c71de634f58f5ec89e66d729e0ffc`.
- The harness establishes the fixed revision first, then replaces only `JaccardSimilarity.java` with the exact parent version. It asserts that no test changed.

Run from the Sieve repository with Maven and Java 17+:

```sh
cargo test --locked --test upstream_regressions -- --include-ignored --test-threads=1
```

`IMPACT_MAVEN` chooses the Maven executable. The fixture pins the compiler plugin, Surefire, JUnit and AssertJ versions and compiles for Java 17. `toolchain.log` records the actual Maven and Java runtime versions. Java and Maven distributions themselves are supplied by the caller.

No upstream repository or service is contacted during the regression test. Maven may download the pinned build/test dependencies on first use; after priming its cache, use `SIEVE_UPSTREAM_OFFLINE=1`. Setup's effective-model plugin must also be cached.

The harness verifies three full shadow executions:

1. Fixed production plus unchanged upstream tests passes and reports no failure-retention evidence.
2. Reverting only the production fix produces two real failing classes, both retained by the ordinary class selector.
3. `SIEVE_BROKEN_SELECTOR=1` proposes no tests; observation still executes the full suite and reports both missed failing classes.

JSON observations, build logs, the recorded toolchain and a compact evidence summary remain in `target/upstream-regressions/text158/`. Set `SIEVE_UPSTREAM_RESULTS` to retain them elsewhere. Ordinary `cargo test` verifies the vendored upstream blobs without running Maven.

Verified on 2026-10-12 with Maven 3.9.11 and Eclipse Adoptium Java 22.0.2: the fixed fixture passed all three test classes; the reverted source failed the two unchanged Jaccard classes; ordinary selection proposed those two classes; the deliberately broken selector reported both as missed. The unrelated Hamming test class ran in every full shadow execution.

This is one real regression case exercising static selection. It is not a performance benchmark, a runtime-record validation, a full Commons Text build, or evidence of general failure-retention accuracy. The upstream assertions are unchanged; the small standalone build deliberately omits unrelated upstream modules and release tooling.
