# Sieve: Java and Kotlin test impact

A Rust CLI that runs tests in changed JVM modules and their transitive dependents.
It supports Java, Kotlin/JVM, and mixed projects using Maven or Gradle. The selector,
fixture manager, validation oracle, and tests are all Rust.

Selection is conservative and module-level. There is no class-level analysis or
runtime recording agent yet. Build configuration changes or uncertain Git history
trigger the full suite.

## Install and set up a project

Install from this checkout:

```bash
cargo install --path . --locked
```

Or install directly from GitHub:

```bash
cargo install --git https://github.com/preacherxp/sieve --locked
```

From an existing Java or Kotlin project:

```bash
java-test-impact init
java-test-impact run --workspace . --base origin/main
```

Use your comparison branch, such as `origin/master`, in place of `origin/main`.
Setup detects Maven or Gradle, prefers an existing build wrapper, and discovers the
declared module dependencies through Maven's effective models or Gradle's project model.
It writes `impact.json`. Maven also receives small test-skipping profiles in its
module POMs. Gradle uses a bundled init script, so both `build.gradle` and
`build.gradle.kts` work without build-file edits. Commit the generated configuration
and POM changes. Setup refuses to overwrite an existing `impact.json`.

Optional overrides:

```bash
java-test-impact init --workspace /path/to/project --tool gradle --executable /path/to/gradle
java-test-impact run --workspace /path/to/project --base origin/main --executable /path/to/gradle
```

Automatic setup currently supports a single JVM package or direct child modules
whose directory names match their module names. Nested/custom module layouts,
Gradle composite builds, Android, and Kotlin Multiplatform are not supported by
automatic setup. The tool reports unsupported layouts instead of guessing.
Run setup with the same Maven profiles and build environment used by CI. Keep
`impact.json` complete when adding dependencies, including runtime/resource edges.
Custom dependency substitution and dependencies introduced through external artifacts
need manual graph review; automatic setup collects declared inter-project edges.

Prerequisites: Rust 1.92+ to install/build the CLI, Git for change detection, and the
JDK/build tool required by your project. The Gradle adapter requires **Gradle 8.11+**.
The samples use JDK 17, Maven 3.9.9,
Gradle 8.12.1 in CI (the existing wrapper is 8.13), Kotlin 2.2.21, JUnit 5.11.4,
and Spring 6.1.16. Initial builds need access to the normal dependency repositories.

## How selection works

`impact.json` describes the build tool and direct dependencies:

```json
{
  "tool": "maven",
  "modules": {
    "pricing": [],
    "checkout": ["pricing"],
    "runtime": []
  }
}
```

A single-module package uses `".": []`. The conventional source layout is
`<module>/src/...`, including `src/main/java`, `src/main/kotlin`, tests, and resources.

1. Find the merge base between `--base` and HEAD.
2. Collect committed, staged, unstaged, deleted, renamed, and untracked changed paths.
3. Select changed source/resource modules and follow reverse dependency edges.
4. Run every unit/integration test in those modules through the native build tool.

For the samples, pricing changes select pricing and checkout: **8 classes / 9 test
invocations**. Checkout changes select **4 classes / 5 invocations**. Runtime changes
select **5 integration classes**. Kotlin code participates in the same graph as Java.

Root `README.md`, `VALIDATION.md`, and `docs/` changes select NONE. Other changes
outside recognized source directories, including build scripts, dependency versions,
configuration, and shared repository inputs, select ALL. An unavailable base or Git
history also selects ALL. Invalid configuration fails explicitly.

```bash
# Preview a selection without executing tests.
java-test-impact select --workspace projects/maven --base origin/master

# Execute selected tests and save the decision before the build starts.
mkdir -p validation-results
java-test-impact run --workspace projects/maven --base origin/master \
  --output validation-results/maven-selection.json

# Always run the full suite.
java-test-impact run --workspace projects/gradle --full
```

Omitting `--base` also requests a full run. Put selection output in an ignored
folder or outside the project so it does not become an untracked build input.
The runner propagates build/test failures and accepts additional build arguments
after `--`. It starts with `clean` to prevent stale XML reports; NONE runs only
`clean`, without compilation or tests. No baseline metadata or selection cache is
needed for this algorithm. Ordinary Maven/Gradle commands still run all tests.

## GitHub CI

The repository has explicit, sequential stages on pushes, pull requests, and manual runs:

1. **Rust and selector checks:** formatting, Clippy, unit checks, and all scenario
   selections for both build tools.
2. **Selected tests:** Maven and Gradle jobs use the PR base or previous push SHA.
   Missing history falls back to ALL. A controlled mutation also proves that each
   adapter executes the expected subset and catches its known failures.
3. **All tests:** separate Maven and Gradle jobs execute the full suite on the same
   revision. This stage still runs if the selective stage fails, unless cancelled.

Each stage uploads its own selection and JUnit reports and adds its decision to the
job summary. CI uses full Git history, read-only repository permissions, no persisted
checkout credentials, and no secrets for pull requests.

The weekly/manual `fixtures.yml` workflow validates installation and runs every
mutation twice: once with the full suite and once with selective execution. Set the
Rust check and all four Java/Kotlin job checks as required in branch protection if
you want merges to require both test stages.

Workflows must live at the repository root under `.github/workflows/`.

## Fixture benchmark

`projects/maven` and `projects/gradle` are equivalent three-module Java/Kotlin builds.
Their sources and resources are checked for byte-for-byte parity. Each baseline has
**13 test classes / 14 invocations**, including one parameterized Java test and one
Kotlin test that calls Java production code.

`scenarios.json` is the independent oracle: mutations, minimum required tests, and
expected failing classes are predefined. The Rust selector never reads it.

| Scenario | Behavior exercised |
|---|---|
| tax-transitive | Leaf change propagates across modules and into Kotlin |
| calculator | Shared Java calculation affects downstream Java/Kotlin tests |
| unrelated | Independent currency behavior |
| unused | Unused production code |
| gateway-implementation | Interface implementation |
| inherited-fixture | Shared test superclass |
| changed-test / new-test | Modified tests and tests absent from the baseline |
| reflection / async | Reflective calls and worker threads |
| spring-bean / spring-wiring | Injection and configuration changes |
| resource / service-provider | Classpath resources and ServiceLoader |
| delete-class | Class deletion with a compilable consumer replacement |
| docs-only | Documentation-only changes |
| dependency-change | Conservative dependency invalidation |
| kotlin-source | Kotlin/JVM production code |

Build and validate from the checkout:

```bash
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked

target/debug/java-test-impact fixtures list
target/debug/java-test-impact fixtures verify --tool both
target/debug/java-test-impact fixtures verify --tool both --scenario all
target/debug/java-test-impact fixtures verify --tool both --scenario all \
  --selected --output validation-results/selected

# Installation integration checks require Java 17, Maven, and Gradle on PATH.
cargo test --locked --test cli installs_ -- --ignored
```

`--maven /path/to/mvn` and `--gradle /path/to/gradle` override fixture build tools.
Installer tests also accept `IMPACT_MAVEN`, `IMPACT_GRADLE`, and `IMPACT_TOOL`.
The fixture verifier creates isolated workspaces, checks actual XML execution against
the selected inventory, verifies invocation counts, and rejects compilation errors,
missing tests, extra executed tests, or unexpected failures. It ignores test failures
only inside mutation runs so every module can finish. A mutation PASS means its
expected failures were observed. Logs and JSON results go to `validation-results/`.

To exercise a selector manually:

```bash
java-test-impact fixtures prepare --tool maven --dest /tmp/impact-tax --git
java-test-impact fixtures apply tax-transitive --workspace /tmp/impact-tax
java-test-impact select --workspace /tmp/impact-tax --base HEAD --output /tmp/selection.json
java-test-impact fixtures check-selection tax-transitive --actual /tmp/selection.json
java-test-impact run --workspace /tmp/impact-tax --base HEAD
java-test-impact fixtures reports --tool maven --workspace /tmp/impact-tax
```

The deliberate tax mutation makes the run fail. `prepare` never overwrites an
existing destination; `apply` refuses to stack scenarios. `fixtures --root PATH`
points the fixture commands at a different checkout containing the manifest/projects.

External selectors can export `{"mode":"SUBSET","tests":["pricing:unit:example.TaxRulesTest"]}`,
`{"mode":"ALL","tests":[]}`, or `{"mode":"NONE","tests":[]}`. IDs are
`module:suite:fully.qualified.ClassName`. The included selector exports
`{"mode":"MODULES","modules":["checkout","pricing"]}` with diagnostic fields.
The oracle expands modules using its own inventory. Default checking permits extra
tests but requires all affected tests; `--exact` also rejects extras. Module-level
selection intentionally does not pass every precision check (for example, unused
code still reruns its module).

## Limits

This is a conservative module selector, not a soundness proof for arbitrary JVM
builds. Explicit dependency graphs must include runtime/resource dependencies;
unsupported custom layouts need further adapter work. The deletion fixture also
edits its consumer, so it is not an isolated proof of previous-graph traversal.
Class-level analysis, parallel runtime attribution, generated-code discovery, and
selection-cache invalidation are outside the current implementation.

Build integration follows the native [Maven Kotlin configuration](https://kotlinlang.org/docs/maven-configure-project.html),
[Gradle Kotlin/JVM support](https://kotlinlang.org/docs/gradle-configure-project.html),
and [GitHub PR checkout semantics](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#pull_request).
