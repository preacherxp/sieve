# Sieve: Java and Kotlin test impact

A Rust CLI that runs tests in changed JVM modules and their transitive dependents.
It supports Java, Kotlin/JVM, and mixed projects using Maven or Gradle. The selector,
fixture manager, validation oracle, and tests are all Rust.

Selection is conservative and module-level by default, and builds only the selected
modules and what they depend on. Projects can opt into class-level selection from
compiled bytecode; there is no runtime recording agent. Build configuration changes or
uncertain Git history trigger the full suite.

## Install and set up a project

The CLI is now named `sieve` (formerly `java-test-impact`). Update existing command
invocations after reinstalling. Run `sieve refresh` for existing installations,
review the graph/POM diff, and commit it. Legacy configurations select the full
suite until refreshed.

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
sieve init
sieve run --workspace . --base origin/main
```

Use your comparison branch, such as `origin/master`, in place of `origin/main`.
Setup detects Maven or Gradle, prefers an existing build wrapper, and discovers the
declared module dependencies through Maven's effective models or Gradle's project model.
It writes `impact.json`. Maven also receives per-module Surefire/Failsafe
`skipTests` properties in its POMs; test compilation remains enabled so downstream
test JARs still work. Existing default profiles remain active. Explicit execution
skip overrides in local build sections are rejected during setup; external parent
and profile overrides still require manual review. Gradle uses a bundled init script, so both `build.gradle` and
`build.gradle.kts` work without build-file edits; the adapter is compatible with the
configuration cache. Commit the generated configuration
and POM changes. Setup refuses to overwrite an existing `impact.json`.

Optional overrides:

```bash
sieve init --workspace /path/to/project --tool gradle --executable /path/to/gradle
sieve run --workspace /path/to/project --base origin/main --executable /path/to/gradle
```

Automatic setup currently supports a single JVM package or direct child modules
whose directory names match their module names. Nested/custom module layouts,
Gradle composite builds, Android, and Kotlin Multiplatform are not supported by
automatic setup. The tool reports unsupported layouts instead of guessing.
Run setup with the same Maven profiles and build environment used by CI. Keep
`impact.json` complete when adding dependencies, including runtime/resource edges.
Run `sieve refresh --workspace PATH` after build changes,
using the same profiles/properties as CI (for Maven, for example, `-- -Pci`).
Refresh preserves additional declared edges between surviving modules; review
obsolete edges manually. A fingerprint of conventional workspace build inputs
forces `ALL` when the graph may be stale, including after the build edit was
committed. It cannot detect changes to external models, environment variables,
or undeclared runtime dependencies.
Custom dependency substitution and dependencies introduced through external artifacts
need manual graph review; automatic setup collects declared inter-project edges.

Prerequisites: Rust 1.92+ to install/build the CLI, Git for change detection, and the
JDK/build tool required by your project. The Gradle adapter requires **Gradle 7.6.3+**.
The default samples use JDK 17, Maven 3.9.9, Gradle 8.12.1 in CI (the existing
wrapper is 8.13), Kotlin 2.2.21, JUnit 5.11.4, and Spring 6.1.16. Initial builds
need access to the normal dependency repositories. The compatibility
workflow checks Java 8, 11, 17, 21, and 25 with representative Maven 3.9/4.0 RC,
Gradle 7.6/8/9, and Kotlin 1.9/2.2/2.3 combinations. Build tool and Kotlin plugin
versions must be compatible with the chosen JDK; see the upstream compatibility
tables linked below. Java 8/11 checks use a small Java-only project because the
main sample's Spring 6 dependency requires Java 17.

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
3. Drop paths matching `ignore`, select changed source/resource modules, and follow
   reverse dependency edges.
4. Build only the selected modules and run their unit/integration tests through the
   native build tool: Maven `verify -pl <selected> -am` (dependencies build with their
   tests skipped), Gradle `:<module>:check` per selected module.

For the samples, pricing changes select pricing and checkout: **10 classes / 12 test
invocations**. Checkout changes select **4 classes / 5 invocations**. Runtime changes
select **5 integration classes**. Kotlin code participates in the same graph as Java.

Changes matching the optional `ignore` globs in `impact.json` select NONE, including
paths inside module source directories such as `src/site/**`. Patterns are relative to
the workspace; a leading `/` anchors them at the repository root.
`*` and `?` stay within one path segment and `**` crosses segments. Without `ignore`,
the default is `["README.md", "docs/**", "/README.md", "/docs/**"]`; an explicit list
replaces it. The samples also ignore the repository's `VALIDATION.md`. Other changes
outside recognized source directories, including build scripts, dependency versions,
configuration, and shared repository inputs, select ALL. Do not ignore build scripts,
`impact.json`, or other test inputs: ignored changes never trigger tests. An unavailable base or Git
history also selects ALL. Invalid configuration fails explicitly.

```bash
# Preview a selection without executing tests.
sieve select --workspace projects/maven --base origin/master

# Execute selected tests and save the decision before the build starts.
mkdir -p validation-results
sieve run --workspace projects/maven --base origin/master \
  --output validation-results/maven-selection.json

# Always run the full suite.
sieve run --workspace projects/gradle --full
```

Omitting `--base` also requests a full run. Put selection output in an ignored
folder or outside the project so it does not become an untracked build input.
The runner propagates build/test failures and accepts additional build arguments
after `--`. It starts with `clean` to prevent stale XML reports; because a scoped Maven
build cleans only its reactor, Sieve also deletes the Surefire/Failsafe report folders
of the other modules. NONE runs only `clean`, without compilation or tests. No baseline metadata or selection cache is
needed for this algorithm. Ordinary Maven/Gradle commands still run all tests.

### Class-level selection

Module-level selection runs every test of a selected module, and a single-module
project has only one. Add `"class_level": true` to `impact.json` to narrow a module
selection to test classes. `run` first compiles the selected modules
(`clean test-compile -pl <selected> -am` on Maven, `clean :<module>:impactCompile` on
Gradle), then reads the class files of every module:

- Edges follow constant-pool references (class entries, descriptors, generic signatures,
  annotations), superclasses and interfaces, dotted string constants such as
  `Class.forName("a.B")` arguments, and Kotlin inline-function source maps. Paths may
  cross unselected modules.
- A changed or affected type affects its subtypes. A changed implementation affects
  callers of its interfaces and superclasses, because they may run it through DI or
  `ServiceLoader`, but not the other implementations. A subtype that also calls through
  its supertype (a decorator) is a caller.
- Compilers copy non-private `static final` constants into callers without a reference.
  When a changed class declares one, every class whose source names the constant or its
  class is affected, and so is every class whose source cannot be found.
- Classes annotated for dependency-injection scanning (Spring stereotypes and
  configuration, Spring Data, JPA, JAX-RS, Jakarta/`javax` inject and CDI, Micronaut,
  Quarkus) are components. When an affected class is a component, every test that
  starts a container (Spring TestContext and Boot test annotations, `@MicronautTest`,
  `@QuarkusTest`, Arquillian, including composed annotations and inherited
  configuration, or a test that boots one itself through `SpringApplication`,
  `SpringApplicationBuilder`, a Spring application context, or Micronaut's
  `ApplicationContext`) is selected in the modules that run, since component scanning
  leaves no reference to follow.
- Test classes reaching a change emit `SUBSET` with their binary names in `tests`. None
  reaching it emits `NONE` with the selected modules: the build compiles and verifies
  without executing tests.

Selection falls back to `MODULES` for a changed non-Java/Kotlin file under `src/`
(resources included), a deleted source, `package-info.java`/`module-info.java`, a source
with no compiled class (such as a Kotlin file whose directory differs from its package),
or unreadable class files. Build inputs still select `ALL`.

The second build skips `clean`. Maven receives `-Dsurefire.excludesFile` and
`-Dfailsafe.excludesFile` listing the unselected test classes, so POM includes and the
unit/integration split stay in effect; the file also repeats Surefire's default
`**/*$*` exclude, which an excludes file otherwise drops. Gradle reads the selected
classes from a file (`-Pimpact.testsFile`) and filters every `Test` task to them and
their nested classes. `select` stays module-level because it does not compile;
`--output` receives the refined decision. `refresh` preserves the flag.

Static analysis still cannot see classes named only in resources, reflection built from
non-constant strings, or scanning by frameworks not listed above. A Spring application
change usually reaches a component, so container tests are selected together with the
unit tests that reach it. Keep full-suite runs on the default branch.
[`samples/bookstore`](samples/bookstore/README.md) demonstrates the savings: an edit
selects 1–3 of its 10 test classes. [`samples/webshop`](samples/webshop/README.md) applies
both levels to five Spring WebFlux services and an end-to-end module, and replays a
history of breaking and fixing commits against the full suite.

## GitHub CI

The repository runs these checks on pushes, pull requests, and manual runs:

1. **Rust and selector checks:** formatting, Clippy, unit checks, and all scenario
   selections for both build tools. In parallel, one job builds the release binary that
   every Java job downloads, instead of compiling Rust in each job. Cargo caches cover
   the jobs that still compile tests.
2. **Selected tests:** Maven and Gradle jobs use the PR base or previous push SHA.
   Missing history or shared build/selector changes select ALL; the selected job
   records that decision and the full-test stage executes the suite once. Ten
   additional jobs verify selected execution for Java, Kotlin, integration, and
   edge-case fixture mutations on both build tools, and class-level selection across
   every mutation. The edge jobs also run the native graph, class-level, and Gradle
   configuration-cache checks.
3. **All tests:** separate Maven and Gradle jobs execute the full suite on the same
   revision, in parallel with the selected stage. Six container tests also run on every
   CI event.

Selected and full Java jobs upload selection and JUnit reports and add decisions to
the job summary. Scenario and container jobs upload their own results. CI uses full
Git history, read-only repository permissions, no persisted checkout credentials,
and no secrets for pull requests.

The weekly/manual `fixtures.yml` workflow also validates installation and runs every
mutation under the full suite and the selector, including known failure detection. If merges must require all test
stages, make the Rust check and Java/Kotlin jobs required in branch protection.

The primary CI runs the compatibility matrix as separate jobs on pushes to the
default branch (`master` here; `main` is also accepted). The matrix can also run
weekly or manually. Its jobs check older and newer JDK, build-tool, and Kotlin releases.
Override the sample Kotlin version with `-PkotlinVersion=...` for Gradle or
`-Dkotlin.version=...` for Maven.

Kafka, Redis, MongoDB, PostgreSQL, MySQL, and RabbitMQ Testcontainers jobs run for
pushes and pull requests. They require Docker and run the tests in
`projects/containers`; each verifies a client can write and read data through its
mapped port.

Workflows must live at the repository root under `.github/workflows/`.

## Fixture benchmark

`projects/maven` and `projects/gradle` are equivalent three-module Java/Kotlin builds.
Their sources and resources are checked for byte-for-byte parity. Each baseline has
**15 test classes / 17 invocations**, including a parameterized Java test, a
validated Java record, and Kotlin sealed results and extensions that call Java code.

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
| changed-test / new-test / delete-test | Modified, added, and independently deleted tests |
| reflection / async | Reflective calls and worker threads |
| spring-bean / spring-wiring | Injection and configuration changes |
| resource / service-provider | Classpath resources and ServiceLoader |
| delete-class | Class deletion with a compilable consumer replacement |
| docs-only | Documentation-only changes |
| dependency-change | Conservative dependency invalidation |
| kotlin-source | Kotlin/JVM production code |
| java-record | Java record value and validation behavior |
| kotlin-sealed | Sealed Kotlin result and exhaustive formatting |

Build and validate from the checkout:

```bash
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked

target/debug/sieve fixtures list
target/debug/sieve fixtures verify --tool both
target/debug/sieve fixtures verify --tool both --scenario all
target/debug/sieve fixtures verify --tool both --scenario all \
  --selected --output validation-results/selected
# The same mutations with class-level selection on the multi-module fixtures.
target/debug/sieve fixtures verify --tool both --scenario all \
  --selected --class-level --output validation-results/class-level

# Installation integration checks require Java 17, Maven, and Gradle on PATH.
cargo test --locked --test cli installs_ -- --ignored
cargo test --locked --test native native_ -- --ignored --test-threads=1
cargo test --locked --test native unsupported_gradle_ -- --ignored
# Docker is required; repeat for Kafka.
IMPACT_SERVICE=Redis cargo test --locked --test native selected_containers_ -- --ignored
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
sieve fixtures prepare --tool maven --dest /tmp/impact-tax --git
sieve fixtures apply tax-transitive --workspace /tmp/impact-tax
sieve select --workspace /tmp/impact-tax --base HEAD --output /tmp/selection.json
sieve fixtures check-selection tax-transitive --actual /tmp/selection.json
sieve run --workspace /tmp/impact-tax --base HEAD
sieve fixtures reports --tool maven --workspace /tmp/impact-tax
```

The deliberate tax mutation makes the run fail. `prepare` never overwrites an
existing destination; `apply` refuses to stack scenarios. `fixtures --root PATH`
points the fixture commands at a different checkout containing the manifest/projects.

External selectors can export `{"mode":"SUBSET","tests":["pricing:unit:example.TaxRulesTest"]}`,
`{"mode":"ALL","tests":[]}`, or `{"mode":"NONE","tests":[]}`. IDs are
`module:suite:fully.qualified.ClassName`. The included selector exports
`{"mode":"MODULES","modules":["checkout","pricing"]}` with diagnostic fields; with
class-level selection, `SUBSET` lists binary class names, which the oracle expands to
every inventory entry of that class in the selected modules. Names without an entry,
such as abstract test fixtures, expand to nothing; the verifier still compares the
executed tests with the expansion.
The oracle expands modules using its own inventory. Default checking permits extra
tests but requires all affected tests; `--exact` also rejects extras. Module-level
selection intentionally does not pass every precision check (for example, unused
code still reruns its module).

## Timing this repository

```bash
cargo build --release --locked
target/release/sieve fixtures benchmark --tool both \
  --scenario gateway-implementation --runs 5 --output validation-results/timing
# Also compare tax-transitive (shared dependency) and docs-only (NONE).
GITHUB_REPOSITORY=preacherxp/sieve bash scripts/ci-cost.sh RUN_ID
```

The benchmark alternates native `clean verify`/`clean check`, Sieve `--full`, and
Sieve selected execution, warms each condition once, and retains five or more raw
samples with median/range. Every sample must pass the independent execution oracle
before a timing summary is emitted. Mutation-only failure-ignore flags are used
solely to collect the complete expected failure set. Dependencies are shared and
workspaces are clean; run benchmarks sequentially without competing JVM builds.

This measures local build execution, excluding checkout, CLI installation, fixture
preparation, and later CI stages. It does not represent cold caches or native task
cache hits. The CI cost script separately sums completed job durations and measures
the span between the earliest job start and latest job completion, including failed
jobs. This repository retains full-suite validation after the selected stage, so
faster selected feedback does not establish faster final CI completion.

## Replaying real history

Fixtures are small, so JVM startup dominates their timings. `replay` measures a real
project instead: configure it with `sieve init` (and optionally `class_level`/`ignore`),
then replay its recent first-parent commits.

```bash
sieve replay --workspace . --commits 30 --run --output /tmp/replay.json \
  -- -Dmaven.test.failure.ignore=true
```

Each commit is replayed in a shared clone: its parent and its tree are each committed
with the workspace's `impact.json` (and Maven adapter), so the compared change is exactly
the commit's. Without `--run`, only selections are recorded. With `--run`, every commit
runs `sieve run --full` and `sieve run --base PARENT` from clean trees with the same build
arguments. The report lists selection modes, test cases, seconds, and **missed
failures**: tests failing in the full run that the selected run did not execute. The
command exits with 1 when any failure was missed. Pass failure-ignore flags so full runs
collect every failure. Logs are kept next to the `--output` file.

See [coverage and remaining cases](docs/test-coverage.md) and
[measurements](docs/performance.md) for evidence and limits.

## Limits

This is a conservative module selector, not a soundness proof for arbitrary JVM
builds. Explicit dependency graphs must include runtime/resource dependencies;
unsupported custom layouts need further adapter work. The deletion fixture also
edits its consumer, so it is not an isolated proof of previous-graph traversal.
Class-level analysis is static: see its section for what it cannot see. Parallel
runtime attribution and selection-cache invalidation are outside the current
implementation.

Build integration follows the native [Maven Kotlin configuration](https://kotlinlang.org/docs/maven-configure-project.html),
[Gradle Kotlin/JVM support](https://kotlinlang.org/docs/gradle-configure-project.html),
[Gradle Java compatibility](https://docs.gradle.org/current/userguide/compatibility.html),
[Maven release requirements](https://maven.apache.org/docs/history.html),
and [GitHub PR checkout semantics](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#pull_request).
