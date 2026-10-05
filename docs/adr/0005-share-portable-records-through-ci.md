---
status: proposed
---

# Share portable test records through the CI cache

Local mode (ADR 0001, ADR 0003) kept records on developer machines, so pull requests in CI still ran every test, and on single-module Spring Boot services static selection cannot narrow them. Records are evidence about bytecode and resource hashes rather than about a commit, so the default branch's records stay valid for any later change: a test is skipped only when everything it executed and read is unchanged. CI therefore shares them: the default branch runs every test with `sieve run --ci --full` and saves the records to the GitHub Actions cache, and pull requests restore them and run `sieve run --ci`.

Records must not depend on the machine that produced them. With `--ci`, the invocation key uses the build tool's file name instead of its path and leaves out `PATH` and `JAVA_HOME` (the JDK's release file still counts), and the agent replaces the workspace, local repository, home, JDK, and temporary directories in test-JVM property values with placeholders and leaves out host identity. Only the records and their evidence are cached; run state such as `last-run.json` describes one machine's build output.

## Consequences

- The default branch keeps running the full suite. A failure that a pull request's selection misses fails at merge, never silently.
- Branch-scoped caches keep pull requests from overwriting the default branch's records. A pull request can still only influence its own runs.
- Runners must share the JDK and operating system family; another JDK reruns everything (`JDK changed`).
- Records from `--ci` and from plain local runs never match each other, so developer and CI records stay separate.
