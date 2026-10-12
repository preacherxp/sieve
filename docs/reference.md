# Sieve reference

Full behavior and flags. See the [README](../README.md) for the overview.

A Rust CLI that runs tests in changed JVM modules and their transitive dependents.
It supports Java, Kotlin/JVM, and mixed projects using Maven or Gradle. The selector,
fixture manager, validation oracle, and tests are Rust; an optional test-JVM agent for
local mode is Java.

Selection is conservative and builds only the selected modules and what they depend on.
Setup uses module selection by default; projects can opt into class-level selection
from compiled bytecode after measuring their test costs.
Single-module Maven or Gradle projects can also opt into local mode, which keeps
per-test runtime records and skips tests whose records show no change (see
[Local mode](#local-mode-test-records)). Build configuration changes or uncertain Git
history trigger the full suite.

## Install and set up a project

The CLI is now named `sieve` (formerly `java-test-impact`). Update existing command
invocations after reinstalling. Run `sieve refresh` for existing installations,
review the graph/POM diff, and commit it. Legacy configurations select the full
suite until refreshed.

Install a prebuilt binary (Linux x86_64/arm64, macOS Apple silicon; no Rust or JDK needed):

```bash
curl -fsSL https://raw.githubusercontent.com/preacherxp/sieve/master/install.sh | sh
```

It installs to `~/.local/bin` (override with `SIEVE_INSTALL_DIR`) and verifies the
release checksum. Set `SIEVE_VERSION=v0.3.0` to pin a release; `sieve --version` prints
the installed one. Releases are built by `.github/workflows/release.yml` when a `v*` tag
matching the version in `Cargo.toml` is pushed.

Or install from this checkout:

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

Automatic setup supports a single JVM package or modules at any depth whose directories
match their place in the build: Maven modules listed by path (`<module>libs/core</module>`)
or below nested aggregators, and Gradle projects whose directories match their paths
(`:services:orders` in `services/orders`). A module is named by its directory,
such as `services/orders`; Maven's skip property joins the segments with dots
(`impact.skip.services.orders`), and Gradle tasks follow the project path
(`:services:orders:check`). Aggregators build nothing themselves and are not modules; a
change to their POM is a build input. Gradle plugin-only included builds inside the
workspace are supported: each project must declare plugins through `java-gradle-plugin`,
or be an empty aggregator. Setup checks the actual included graphs, including nested
includes, and records their directories in `build_logic`. Every change under those
roots selects ALL before ignore rules apply; refresh fingerprints their sources and
resources too. Included builds retain their native tests and task dependencies.
Application composites, external includes, relocated project/source directories, Android,
and Kotlin Multiplatform remain unsupported by automatic setup.
Run setup with the same Maven profiles and build environment used by CI. Keep
`impact.json` complete when adding dependencies, including runtime/resource edges.
Run `sieve refresh --workspace PATH` after build changes,
using the same profiles/properties as CI (for Maven, for example, `-- -Pci`).
Refresh preserves additional declared edges and shared test modules between surviving
modules; review obsolete edges manually. A fingerprint of conventional workspace build
inputs tells when the graph may be stale, including after the build edit was committed,
such as a dependency bump merged without a refresh. A module selection then asks the build
for its current graph, with the profiles, properties, settings, and init scripts passed after
`--`. When every module, edge, and shared test module it reports is in `impact.json`, the
selection stands and its reason asks for a refresh; anything new, or a graph that cannot be
read, selects `ALL`. A configuration without a fingerprint selects `ALL` until refreshed.
The fingerprint cannot detect changes to external models, environment variables, or
undeclared runtime dependencies.
Custom dependency substitution and dependencies introduced through external artifacts
need manual graph review. Automatic setup collects declared inter-project edges, and also
edges the build itself creates: for Maven, a sibling module used as a build plugin, a plugin
dependency, an annotation processor path, or an unpacked artifact, and a sibling directory
named in the effective build configuration (such as a shared OpenAPI specification); for
Gradle, a source set directory inside another project, and files of another project, such
as `project(':a').sourceSets.test.output`. Maven reads the effective models of the whole
reactor in one start.

Prerequisites: Rust 1.92+ and a JDK 17+ `javac` to install/build the CLI, Git for change
detection, and the JDK/build tool required by your project. `build.rs` compiles the
local-mode agent with the first JDK 17+ found in `SIEVE_JAVA_HOME`, `JAVA_HOME`,
`JAVA_HOME_<version>_*`, or `PATH`; `cargo install --no-default-features` builds Sieve
without the agent and without local mode. The Gradle adapter requires **Gradle 7.6.3+**.
The default samples use JDK 17, Maven 3.9.9, Gradle 8.12.1 in CI (the existing
wrapper is 8.13), Kotlin 2.2.21, JUnit 5.11.4, and Spring 6.1.16. Initial builds
need access to the normal dependency repositories. The compatibility
workflow checks Java 8, 11, 17, 21, and 25 with representative Maven 3.9/4.0 RC,
Gradle 7.6/8/9, and Kotlin 1.9/2.2/2.3 combinations. Build tool and Kotlin plugin
versions must be compatible with the chosen JDK; see the upstream compatibility
tables linked below. Java 8/11 checks use a small Java-only project because the
main sample's Spring 6 dependency requires Java 17.

Engine checks in `tests/compatibility_modes.rs` verify native discovery, record selection
after an irrelevant production edit, a relevant planted failure in records and class mode,
and the same failure in an independent native full run. All five cases passed locally on
Temurin 22.0.2 and Maven 3.9.16; the compatibility workflow runs them on JDK 21.

| Engine tested | Surefire | Class selection | Runtime records |
| --- | --- | --- | --- |
| JUnit 6.0.3, nested classes and dynamic tests | 3.6.0 | Verified | Active; irrelevant edit skips both classes |
| Spock 2.4 with Groovy 4 | 3.6.0 | Verified | Active; irrelevant edit skips both classes |
| Kotest 6.0.4 with Kotlin 2.2.21 | 3.6.0 | Verified | Active; irrelevant edit skips both classes |
| TestNG 7.11.0 through JUnit Platform | 3.6.0 | Verified | Active; irrelevant edit skips both classes |
| TestNG 7.11.0 through its legacy provider | 3.5.2 | Verified | Inactive; native tests always run and the diagnostic explains why |

These are focused Maven checks, not a claim covering every engine extension or build-tool
combination. Surefire 3.6 executes TestNG through JUnit Platform
([upstream documentation](https://maven.apache.org/surefire/maven-surefire-plugin/examples/testng.html)).
When native reports contain test classes for which Sieve received no execution-listener
events, Sieve reports those classes as executed and disables its no-build shortcut. This
also covers unsupported test JVMs and providers without silently treating them as recorded.
Run the checks with `cargo test --locked --test compatibility_modes -- --ignored --test-threads=1`;
use `IMPACT_MAVEN` for a Maven executable outside `PATH`.

## How selection works

`impact.json` describes the build tool and direct dependencies:

```json
{
  "tool": "maven",
  "modules": {
    "pricing": [],
    "checkout": ["pricing"],
    "runtime": []
  },
  "shared_tests": []
}
```

A single-module package uses `".": []`. The conventional source layout is
`<module>/src/...`, including `src/main/java`, `src/main/kotlin`, tests, and resources.

1. Find the merge base between `--base` and HEAD.
2. Collect committed, staged, unstaged, deleted, renamed, and untracked changed paths.
3. Drop paths matching `ignore`, select changed source/resource modules, and follow
   reverse dependency edges. A change under `<module>/src/test/` stays in its module
   unless the module is listed in `shared_tests`: other modules see test code only through
   a test artifact.
4. Build only the selected modules and run their unit/integration tests through the
   native build tool: Maven `verify -pl <selected> -am` (dependencies build with their
   tests skipped), Gradle `:<module>:check` per selected module.

`shared_tests` lists the modules whose `src/test` code other modules use: through a Maven
`test-jar` (`<type>test-jar</type>` or `<classifier>tests</classifier>`), a Maven build use
(plugin, processor, unpacked artifact, or directory), a Gradle dependency on a named
configuration of another project, another project's files, or a source directory inside
another project. Setup writes it; Gradle `testFixtures` live in `src/testFixtures` and reach
their consumers like main code. Add a module by hand when its tests reach others in a way
setup cannot see. A configuration without the key treats every module as shared.

For the samples, pricing changes select pricing and checkout: **10 classes / 12 test
invocations**. Checkout changes select **4 classes / 5 invocations**. Runtime changes
select **5 integration classes**. Kotlin code participates in the same graph as Java.

Changes matching the optional `ignore` globs in `impact.json` select NONE, including
paths inside module source directories such as `src/site/**`. Patterns are relative to
the workspace; a leading `/` anchors them at the repository root.
`*` and `?` stay within one path segment and `**` crosses segments. Without `ignore`,
the default is `["**/*.md", "**/*.adoc", "docs/**", "/*.md", "/*.adoc", "/docs/**"]`:
Markdown and AsciiDoc files anywhere in the workspace, `docs/`, and the repository's
top-level documentation. An explicit list replaces it. The default never hides module
sources, such as a Markdown resource or the sources of a module named `docs`. Other changes
outside recognized source directories, including build scripts, dependency versions,
configuration, and shared repository inputs, select ALL. Do not ignore build scripts,
`impact.json`, or other test inputs: ignored changes never trigger tests. An unavailable base or Git
history also selects ALL. Invalid configuration fails explicitly. Submodule updates count
as changes even when `.gitmodules` or Git configuration ignores them.

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

### Explicit native tasks

Repeated `--task` options replace the default goals/tasks. They retain the complete
native task graph; Sieve filters tests without adding `clean`, `-pl`, or module check
tasks. Maven's generated skip properties and Gradle's `Test` adapter still apply.
NONE skips tests while compiling, packaging, non-test checks and report dependencies
continue according to the requested graph. Ordinary flags remain after `--`.

```bash
sieve run --base origin/main --task verify -- -Pci
sieve run --base origin/main --task check --task integrationTest --task jacocoTestReport
```

Sieve clears conventional native test XML before these runs, so report consumers see
this invocation's tests. Coverage remains the native report task's responsibility:
use `--task clean` or fresh coverage destinations when old execution data could survive.
A partial run is not reconstructed full-suite coverage; retain the selection JSON with
its reports. Custom XML directories and non-`Test` Gradle runners need their own adapter.

### Must-run tests

`"must_run": ["com.example.Contract*Test", "com.example.SmokeTest"]` adds matching
classes to ordinary JVM selections, even an unchanged run. Patterns use dotted JVM
class names, with `*` matching any sequence and `?` one character. Nested matches run
the enclosing class. Native includes, excludes, suite assignment and explicit test
requests still bound execution; the policy does not override those user choices.

Static mode may compile additional modules to discover policy classes. With
`class_level: false`, it still keeps every test in the modules originally selected.
With records, configuring a policy disables the no-build shortcut so discovery can
find matching classes on every invocation. `refresh` preserves the policy. It is useful
for checks of remote state; that state is not automatically tracked.

### Shadow observation

```bash
sieve run --base origin/main --observe --output /tmp/shadow.json
```

Observation supports static module/class selection. It computes the proposed classes
before each native suite executes, then runs all tests allowed by the native invocation
once. Explicit `--task` goals, profiles and native filters still define that invocation.
The output retains the proposal and includes actual executed/failed class identities,
`missed_failed_classes`, `failure_retention`, and `would_detect_test_failure`. Maven
proposals are scoped by module so an unrelated module's fallback cannot hide a miss.
Test/build failures propagate, even when native test failures were configured to be ignored.

A passing run reports **no failure-retention evidence** and a null retention ratio.
A build failure without test XML is inconclusive. Observation cannot reveal failures
caused only by subset ordering or shared state, and it does not measure selected-run
wall time. Continue paired replay/full checks for those questions. A pinned independent
upstream regression is available in `fixtures/upstream`.

### Execution receipts

```bash
sieve run --base origin/main --receipt /tmp/passed.json --task verify
sieve run --remaining /tmp/passed.json --task verify
```

The first stage records actual successfully completed classes from native XML. The
second runs the complete requested graph, excluding those classes only when the
receipt's checkout, revision, source contents, invocation, environment, toolchain,
Sieve binary and current suite inputs still match. Suite inputs include compiled
classes/resources, resolved test classpaths, the actual test JVM, and effective runtime
settings. Changed/new test inventory invalidates that suite. Failed, skipped, aborted,
incomplete or unreported classes never count as completed. A failed build invalidates
the output receipt. Missing or mismatched receipts run the full native invocation.

Both stages must use the same tasks and flags. `--remaining` accepts neither a comparison
base nor native explicit test filters. A must-run class always runs again. Receipts are
local, trusted files bound to one absolute checkout; they are not portable CI records.
They do not track remote services or other undeclared external state. Repeated Maven
executions of the same suite and unverifiable extra classpath dependencies receive no
reusable credit. Maven receipts require the embedded extension and cannot combine with
a conflicting extension override or generated-source comparison. The feature supports
static Maven/Gradle mode; splitting runs can change ordering, so retain independent full
checks while evaluating it.

### Class-level selection

Module-level selection runs every test of a selected module, and a single-module
project has only one. Add `"class_level": true` to narrow a module selection to test
classes. It is opt-in because static analysis has blind spots (below); `refresh` preserves
that choice. `run` selects inside the one build that runs the tests, once the selected
modules compiled. On Gradle, the init script's `impactSelect` task runs after they compile
and before their `Test` tasks, and calls `sieve classes`. On Maven, a Sieve core extension,
loaded with `-Dmaven.ext.class.path`, decides each module right before Surefire or Failsafe
runs its tests, when the module and every module it depends on have compiled: their classes
are the only ones its tests can reach. Changes in other modules do not count for it. The
selector reads the class files of every module it considers:

- Edges follow constant-pool references (class entries, descriptors, generic signatures,
  annotations), superclasses and interfaces, class names spelled in string constants
  (`Class.forName("a.B")`, `@MethodSource("a.B#cases")`, `T(a.B)` in SpEL), and Kotlin
  inline-function source maps. Paths may cross unselected modules. A source maps to the
  classes of the package it declares, even when its directory differs.
- A changed or affected type affects its subtypes. A changed implementation affects
  callers of its interfaces and superclasses, because they may run it through DI or
  `ServiceLoader`, but not the other implementations. A subtype that also calls through
  its supertype (a decorator) is a caller.
- Compilers copy non-private `static final` constants into callers without a reference.
  When a changed class declares one, every class whose source names the constant or its
  class is affected, and so is every class whose source cannot be found. The search
  repeats for the constants of those classes, which may be computed from the changed one.
- Classes annotated for dependency-injection scanning (Spring stereotypes and
  configuration, Spring Data, JPA, JAX-RS, Jakarta/`javax` inject and CDI, Micronaut,
  Quarkus) are components. When an affected class is a component, every context test
  (one that starts an application context: Spring TestContext and Boot test annotations, `@MicronautTest`,
  `@QuarkusTest`, Arquillian, Weld JUnit, `@HelidonTest`, Spring Modulith
  `@ApplicationModuleTest`, including composed annotations and inherited
  configuration, or a test that boots one itself through `SpringApplication`,
  `SpringApplicationBuilder`, a Spring application context, or Micronaut's
  `ApplicationContext`) is selected in the modules that run, since component scanning
  leaves no reference to follow.
- Spring Boot test slices scan only some kinds of components, so a reached component
  selects a slice test only when the slice scans its kind. `@WebMvcTest`/`@WebFluxTest`
  scan controllers (only the listed ones when `controllers`/`value` is set), controller
  advice, Jackson components, and components extending web, conversion, servlet, or
  security types. The data slices (`@DataMongoTest`, `@DataJpaTest`, `@JdbcTest`, and the
  other Spring Data slices) load Spring Data and JPA types. `@JsonTest` and
  `@RestClientTest` load Jackson components. The application class, auto-configuration,
  and `@ConfigurationProperties` join every slice. Slices with `includeFilters` or
  `useDefaultFilters`, other slices, and every slice when the application declares its
  own `@ComponentScan` count as full context tests. Both the Spring Boot 3 and Spring
  Boot 4 annotation packages are recognized.
- Tests that find classes by scanning (ArchUnit, Spring Modulith `ApplicationModules`,
  JUnit suites selecting packages or class-path resources, Cucumber, ClassGraph,
  Reflections) leave no reference to what they check, so any change selects them.
- A class named in a text file under `src/` (Spring XML, logging or mapping configuration,
  a service file of a library type such as a JUnit extension) may run wherever the file is
  read. When such a class reaches a change, selection falls back to `MODULES`, as for a
  changed resource. `name` attributes such as logger names do not count, and service files
  of project types are followed through their supertypes. Classes named in Spring's
  `META-INF/spring.factories` and `META-INF/spring/*.imports` join every context and slice
  instead.
- Test classes reaching a change emit `SUBSET` with their binary names in `tests`, and
  `reasons` shows how each reaches it, such as
  `a.ServiceTest → a.Service → a.Api, implemented by a.Impl (changed)`. None reaching it
  emits `NONE` with the selected modules: the build compiles and verifies without
  executing tests.

Selection falls back to `MODULES` for a changed non-Java/Kotlin file under `src/`
(resources included), a deleted source, `package-info.java`/`module-info.java`, a source
with no compiled class, a class named in a resource that reaches the change, or unreadable
class files. Build inputs still select `ALL`.

Resources that tests reach only through the code generated from them, such as OpenAPI
specifications, can be declared as workspace-relative globs (Maven only):

```json
{ "class_level": true, "generated": ["src/main/resources/openapi/**"] }
```

When a matching file changes, `run` checks out the merge base in a temporary Git worktree
and runs `generate-test-sources` there while the workspace compiles. It then compares
`target/generated-sources` and `target/generated-test-sources`, ignoring
`@Generated(...)` lines and annotation-processor output, and treats the generated
sources that differ as the changed sources. The selection lists them under `generated`.
Only declare inputs that are not also read at runtime or by tests: a declared input
reaches tests solely through its generated classes. A removed generated source, or failed
generation at the base, keeps the module selection.

On Maven, Surefire and Failsafe read an excludes file listing the module's unselected test
classes, after the excludes file the POM configures, so POM includes and the
unit/integration split stay in effect; the file also repeats Surefire's default `**/*$*`
exclude, which an excludes file otherwise drops. When the extension cannot apply it, the
module runs every test and the build log says so. A changed `generated` input, or a build
that loads its own extensions through `maven.ext.class.path`, compiles first
(`clean test-compile -pl <selected> -am`) and starts Maven again with
`-Dsurefire.excludesFile` and `-Dfailsafe.excludesFile`; that second build skips `clean` and
recompiling the main classes (`-Dmaven.main.skip=true`), and plugins that post-process
classes in place run again on the already processed output. Gradle's `Test` tasks read the
selection when they start and filter to the selected classes and their nested classes; a
selection without classes skips them, and the selection file is a task input, so a cached
result never stands for another selection. `select` stays module-level because it does
not compile; `--output` receives the refined decision before any test runs on Gradle, and
when the build ends on Maven. `refresh` preserves the flag.

Static analysis still cannot see classes named in resources outside `src/`, reflection
built from non-constant strings, context tests whose composed annotation lives in a
library jar, or scanning by frameworks not listed above. A Spring application
change usually reaches a component, so context tests are selected together with the
unit tests that reach it. Keep full-suite runs on the default branch.
[`samples/bookstore`](../samples/bookstore/README.md) demonstrates the savings: an edit
selects 1–3 of its 10 test classes. [`samples/webshop`](../samples/webshop/README.md) applies
both levels to five Spring WebFlux services and an end-to-end module, and replays a
history of breaking and fixing commits against the full suite.
[`samples/selective-performance`](../samples/selective-performance/README.md) reproduces
the single-module case where module selection saves no work, then measures the
opt-in class selection against native full and module-selected runs.

## Local mode: test records

For developers on a single-module Maven or Gradle project whose test JVM runs Java 17+, local mode
replaces static selection with evidence from earlier runs. It pays off most for Spring
context tests: framework dispatch (Kafka listeners, HTTP handlers) leaves no bytecode edge
from a test to the code it runs, so static analysis runs all of them for any component
edit. Opt in explicitly, then run from the project directory before and after every edit:

```bash
sieve run --records  # first time: every test runs and leaves a record; then only what edits affect
```

`--records` enables local mode with or without `impact.json`; without one it uses
defaults and treats OpenAPI specifications (`<inputSpec>`) as generated inputs.
`"records": true` in `impact.json` also enables it for subsequent `run` commands,
including `--base` and `--full`. Keep that setting out of shared CI configuration:
use explicit `--records` on developer machines, and module or class selection in CI.
Without either opt-in, selection is static and requires `impact.json`:

```bash
sieve run --records --base origin/main   # static fallback after a green run with the same invocation
sieve run --records --full               # run everything, still recording
```

`sieve run --records` starts Maven once with an incremental `verify`. It
adds `clean` only when stale output is possible (a file deleted
or renamed since the last run, a build input edit, a `generated` input edit, no
earlier run, or a failed/interrupted previous run). A content edit that preserves file metadata also cleans to prevent stale
compiler output. When nothing changed since the last passing run, including the command,
environment, and Sieve version, it reports `NONE` without starting Maven. Build arguments
after `--` are appended; if they name goals or phases, they replace `verify`.
Concurrent wrapper runs in one workspace are serialized through an OS file lock.

Every `run`, in local and static mode, ends with a summary below the build output, which
`--output` also records under `summary`. Test classes and cases that ran come from the
build's XML reports. A partial run without a complete suite inventory reports an unknown
total instead of claiming every test ran. When `GITHUB_STEP_SUMMARY` is set, the summary
is also written to the job page with selection mode, reason, and selected/total modules
for testing.

Historical durations in `.sieve/timings.json` estimate the **serial test execution time
avoided** for skipped classes. They do not measure wall-clock savings: parallel tests,
shared Spring contexts, container startup, and compilation change that relationship.
Missing durations remain unknown. Static selection keeps durations only where local
mode already created `.sieve/`. Use [replay](#replaying-real-history) to measure both
full and selected build wall time; its report excludes inconclusive comparisons.

In JSON, `suite_known: false` means class/case totals reflect current or historical
observations, not a complete inventory of the current suite.
`serial_test_seconds_avoided` names the historical estimate. The compatibility fields
`saved_seconds` and `estimated_full_seconds` remain available as legacy additive
estimates; they are neither measured wall-clock savings nor full-build forecasts.

On Gradle, `sieve run --records` starts one build on the Gradle daemon and never adds `clean`:
Gradle's incremental compilation removes the output of deleted sources itself. By default
it runs every `Test` task (`impactTests`, added by Sieve's init script) without linters or
packaging; task names after `--` replace it, such as `sieve run --records -- check`. The init script
adds the agent to the `Test` tasks' JVMs only, never to the Gradle or Kotlin daemons.
Gradle's own `--tests` filter always runs the named tests, through the `test` task when
no task is named.

The embedded agent reaches every test JVM through `JDK_JAVA_OPTIONS`, so POM `argLine`
settings (including JaCoCo's) are kept; the agent ignores Maven's own JVM. At JUnit
Platform discovery it asks `sieve decide` which test classes to drop, and after the run
it hands `sieve record` what each top-level test class executed: every project method,
whether loaded from the build's output directories or from a jar the build packaged in the
workspace (Failsafe tests the packaged jar), and every workspace file it opened, looked up, or listed through the JDK's file APIs
(including class-path resource lookups, which also record resources that do not exist
yet, and directory listings). Spring handler and intercepted method attempts also count,
including requests rejected before the body and calls answered by an interceptor.
Test property files named by `@TestPropertySource` count
as read while the context starts. Work done while a Spring
test context or Spring Boot application starts counts for every test class that uses the
context, including classes that ran after it was cached. Lazy singleton initialization
and `FactoryBean` product creation also count for every test class using the owning
Spring test context. Prototype beans retain their test's evidence; factory products
conservatively count with the context even when the factory does not cache them.
Work that outlives a test class,
such as a message it sent and did not await, counts for that class. Records live in
`.sieve/`, which ignores itself in Git and survives `mvn clean`. They accumulate across a
test class's passing runs until the class itself changes.

A test class is dropped when its last run passed and none of the following changed since:

- the test class, its nested classes, the JDK, or Sieve's embedded agent;
- build inputs (including parent POMs outside the workspace), unless the edit only bumped
  dependency versions: the set of artifacts on the test class path is unchanged and only
  jar contents differ. Then only the test classes that ran code of a changed jar, or read
  its entries, rerun, together with the tests whose context startup did (`Changed
  dependency: jackson-databind`). An added, removed, or swapped artifact, a jar without
  classes, or a changed test-JVM flag (`argLine`) still reruns every test; so does any
  other build edit that changes compiled classes or test-JVM properties. A build edit that
  changes none of these, such as a Surefire option that only affects reporting, is treated
  as behavior-neutral;
- Maven or Gradle invocation arguments, test-JVM arguments, stable JVM system properties, or declared environment inputs
  (`"record_env": ["SERVICE_MODE"]` in `impact.json`). Properties that change with every
  commit without changing behavior can be excluded with `"record_ignore_properties":
  ["git.*", "build.number"]` (names, or prefixes ending in `*`); the `jgitver.*`
  properties that jgitver passes into the test JVM are always excluded. A test that reads an
  excluded property is not rerun when it changes. When records are invalidated this way,
  the reason names the changed properties; `.sieve/contexts/` keeps their names with a
  digest of each value, never the values;
- a method it executed (bodies are hashed without debug information, with constant-pool
  references resolved, so comment edits and renumbered constants change nothing);
- a file it read, looked up, or listed;
- the shape (signatures, fields, annotations, supertypes, static initializer) of a class
  it used, of that class's project supertypes and subtypes, of classes whose fields it
  used, and of the annotation types it carries; private methods without annotations are
  left out, since only the class's own code calls them;
- the Spring wiring of a component it uses, or, for changes that can act anywhere in the
  context, of a component its context test loads (see below);
- an added class named by a string constant in a class it used.

A test *uses* a class when it ran code of that class other than its constructors and
static initializer, or used its fields. A Spring context constructs every component it
loads, so constructing one is not using it: a constructor or static initializer whose
bytecode only stores values into the class's own fields (calling nothing but JDK value
types, collections, `java.time`, and null checks) reruns only the tests that use the
class. Other constructors count like any executed method.

Component wiring is compared in parts. A new constructor dependency, or an annotated
member whose annotations act on calls to it (`@Transactional`, security, caching,
validation, `@Value`, Spring Data `@Query`, JPA and Jackson mappings, project annotations
that carry only such meta-annotations), reruns the tests that use the component; for an
interface such as a repository, the tests that use classes calling the changed members.
An added component with such annotations, and no library supertype a framework looks for,
reruns the tests that use its project supertypes, their other implementations, or their
callers. Request mappings also rerun the users of controllers whose paths may overlap the
changed ones. Any other change, such as a listener, `@Bean` method, lifecycle callback,
aspect pointcut, class annotation, supertype, or a removed component, reruns every context
test that loads the component, as in class-level selection.

On an existing concrete method, a value edit to an annotation that acts only on calls
selects tests that executed or attempted that method, or used a class calling it. For example, changing
one handler's `@RequestParam` name leaves tests of another handler unchanged. This requires
unchanged annotation types, constructors, fields, signatures, and class declaration;
unchanged request mappings on the method do not widen the edit. Annotation additions and
removals, changed mappings, unknown annotations, and older snapshots without this evidence
retain the wider class or context selection. The context startup check still applies.

A changed Spring Boot configuration file (`application*` or `bootstrap*`, YAML or
properties) is compared key by key: comment and formatting edits change nothing, and a
changed key reruns the tests that use a project class naming it, in a placeholder such as
`@Value("${clinic.reminders.window}")`, as a key constant, or as the prefix of a
`@ConfigurationProperties` class. A key that no project class names, such as
`spring.datasource.*`, is read by the framework, and every test that read the file runs;
so does a file the reader cannot follow (anchors, tabs, multi-line plain scalars). A
Flyway migration directory that only gained versioned migrations after the newest one,
containing only non-unique indexes, sequences, comments, and tables without foreign keys,
reruns nothing.

Each of these narrowed changes could still stop a context from starting. When none of the
selected tests starts a full context and no such test has passed since the change, the
one with the fewest recorded methods runs as a startup check, with the reason `Startup check for changes it does
not use: …`.

Without a passing record, a test class runs. After a passing wrapper run under the same
invocation, `--base` can use class-level analysis to drop unrecorded tests that reach
no change since that base, which is assumed green. New or changed invocations run them.
Tests that failed run until they pass. Explicitly requested tests (`-Dtest`,
`-Dit.test`, Gradle's `--tests`) always run. Concurrent classes connected through
overlapping execution share method, file and Spring-context evidence. A relevant change
therefore reruns the whole connected group; separate groups can still be skipped. Cached
contexts carry shared startup evidence to later users. Test execution is not serialized.
A failed probe, uninstrumented project or changed class files retains conservative fallback. A class that failed in the build reports, or
ran without the agent reporting it in a failed build, counts as failed, and a build whose
test failures were ignored does not count as passing. `--output` gives a reason for
every test class, and lists dropped ones under `skipped`; build reports omit them.

Plain Maven can use test records through the agent option:

```bash
JDK_JAVA_OPTIONS="$(sieve env --workspace .)" mvn verify
```

Plain Maven runs unrecorded tests; it has no known previous wrapper invocation for
static fallback. Its agent files are isolated from wrapper run state.

[docs/local-mode-adoption.md](local-mode-adoption.md) walks a service through
adoption, measurement, and the test-setup changes that pay off most.

Records assume that every path a test can take has shown up in one of its passing runs
since the test class last changed, and that tests do not depend on what earlier tests
left behind. Not tracked: undeclared environment variables, external services,
floating Docker image tags, and dynamically attached/native agents. Startup `-javaagent`
jar contents are fingerprinted, so replacing a jar at the same path invalidates records
and the no-build shortcut. JVMs
older than Java 17 (back to Java 8) load the agent but keep it inactive, so every test
runs; so do class files newer than the vendored ASM can read. Surefire
with `useSystemClassLoader=false` is not supported. Multi-module projects and consumer
CI use module- or class-level selection. Plain Gradle runs without Sieve get no records.

Inactive diagnostics distinguish explicit native test requests, missing discovery
decisions, and classes that appear in native XML without execution-listener events.
Unobserved executions disable the no-build shortcut; they are never treated as trusted
records. Inspect `.sieve/agent.log` and native build output for provider/JVM startup
errors. Supported record collection needs Java 17+ and JUnit Platform lifecycle hooks.

The agent is Java (`agent/`), compiled by `build.rs` for Java 17 against small API stubs,
with a vendored and relocated copy of ASM (`agent/asm`, BSD-3-Clause), and embedded in the
binary; `sieve` extracts it to `SIEVE_CACHE_DIR` (default: the user cache
directory) and compares it with the embedded bytes on every use. The fixture
`projects/records` covers plain, resource-reading, and Spring Boot tests:

```bash
cargo test --locked --test records -- --include-ignored --test-threads=1
# Add IMPACT_OFFLINE=1 to run Maven offline, IMPACT_MAVEN=/path/to/mvn for another Maven.
# The Gradle case uses Gradle 8.14+ from PATH, or IMPACT_GRADLE=/path/to/gradle.
```

### Local speed-ups

Local speed-ups are opt-in with `--with NAME[,NAME]`, or by default for the team with
`"with": [...]` in `impact.json`; `--without` overrides both. `jgitver` is the exception:
local and `--ci` runs of a Maven build that loads jgitver skip it by default.
`--output` lists each one's state under `speedups`:

- `reuse`: `TESTCONTAINERS_REUSE_ENABLE=true`, so containers declared `withReuse(true)`
  survive between runs.
- `jgitver`: `-Djgitver.skip=true` when `.mvn/extensions.xml` loads jgitver, on by default.
  jgitver stamps the commit into the project version and the test JVM's `jgitver.*`
  properties, so every commit would change filtered resources that embed the version, such
  as a Spring `application.yml`, and rerun every test that reads them. Tests do not depend
  on the version; `--without jgitver` keeps it. The default and an explicit `--with jgitver`
  key the same records.
- `repackage`: skip Spring Boot packaging with `-Dspring-boot.repackage.skip=true`.
  `build-info` still runs because applications read it; its timestamp is not hashed.
- `mvnd`: the daemon forks test JVMs with its own environment,
  which may not carry the agent option. It applies only without `--executable`.

A JDK AOT class cache for the test JVM is not offered: JDK 25 refuses to create one while
the class path holds a non-empty directory, and a Surefire class path always holds
`target/classes`.

### Measuring feedback time

An edit catalog times typical edits on a warm tree. Each edit is a search-and-replace
(`search`, `replace`), a new file (`content`), or a move (`rename_to`, optionally with
`search`/`replace`), tagged with one of the kinds `pom`, `rename`, `new-class`,
`structural`, `config`, `listener-body`, `body`, `resource`, `new-test`, `test-config`,
`fixture`, `test`, `other`, and `docs`:

```json
{
  "samples": 3,
  "build_args": ["-Dbookstore.setupMillis=200"],
  "edits": [
    {"name": "Tax body", "kind": "body", "file": "src/main/java/a/Tax.java",
     "search": "return net;", "replace": "BigDecimal gross = net;\n        return gross;"}
  ]
}
```

```bash
sieve catalog --workspace . --catalog catalog.json --output /tmp/catalog [--plant] [--levers]
sieve classify --workspace . --commits 50
```

`catalog` copies the workspace to a temporary Git repository (`--in-place` uses the
workspace itself, and a journal in `.sieve/` undoes an interrupted run the next time).
After a warm-up it alternates, per sample, a native full build without `clean`, `sieve run
--full`, and each edit applied to HEAD with `sieve run --base HEAD`, reverting the edit
and letting Sieve settle after each. It writes `catalog.json` (raw samples) and
`summary.md`: median and range per edit, and a phase breakdown (`sieve`, `maven_start`,
`compile`, `test_jvm_start`, `tests`, `after_tests`) taken from timestamped build output,
which adds up to each run's wall-clock time. Spring Boot and Testcontainers startup times
are reported alongside. `classify` assigns each of the last N first-parent commits the
most expensive kind among its changed paths, without building anything; `catalog` uses
those weights for a weighted feedback time per commit, next to the native build and
`--full`. Catalogs for private projects can live anywhere. `--levers` times every edit
again with each speed-up toggled. Examples: `samples/bookstore/catalog.json`,
`projects/records/catalog.json`.

`--plant` plants a bug in each edited method body: a negated condition, a flipped
comparison or boolean, swapped arithmetic, a default return value, or a dropped call,
taking the first mutant that compiles. It then runs the selected build and the full build,
and reports a **missed failure** for every test that fails in the full run but not in the
selected one; any missed failure exits with 1. `SIEVE_BROKEN_SELECTOR=1` is a test hook
that makes the selector drop everything, which planted bugs must catch.

`sieve replay --walk --commits N [--plant]` evaluates local mode on real history: one
working tree follows the last N first-parent commits in order, carrying records and build
output forward (Sieve cleans only on its own triggers). Each commit runs `sieve run --base
PARENT` and then a native full build as the reference, and optionally a planted bug in the
commit's changed code. The report lists feedback time, phases, `clean` runs, and missed
failures per commit and in total; any missed failure exits with 1. The walk switches local
mode on for any single-module Maven or Gradle project, and leaves the workspace untouched.

### Records in CI

`sieve run --ci` is an experimental implementation for evaluating record portability.
It is not supported for consumer CI rollout; use module or class selection and retain
full-suite validation there. Local mode remains a developer-machine feature.

The flag enables local mode with a different invocation key. The
invocation key uses the build tool's file name instead of its path, and leaves out `PATH` and
`JAVA_HOME`; the JDK itself is still compared. In the test JVM's properties, the workspace,
local Maven repository, home, JDK, and temporary directories become placeholders, so the
test class path still names every dependency jar, and the host's identity (`user.name`,
`os.version`, library paths, proxy hosts) is left out. Records from `--ci` and plain local
runs never match each other. `--ci --full` runs every test and refreshes this experimental
record set. These mechanics do not remove local mode's assumptions about independent
tests, observed execution paths, or undeclared external inputs.

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
   revision, in parallel with the selected stage. Six Testcontainers tests also run on every
   CI event.

Selected and full Java jobs upload selection and JUnit reports and add decisions to
the job summary. Scenario and Testcontainers jobs upload their own results. CI uses full
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

## JavaScript and TypeScript (Jest, Vitest)

Single-package projects tested with Jest or Vitest select test files through the runner's
own import graph ([ADR 0006](adr/0006-select-javascript-tests-through-the-runner.md)).
Workspaces (monorepos) are not supported yet.

```bash
npm ci                                  # init and run load the project's own runner
sieve init                              # detects jest or vitest in package.json
sieve run --workspace . --base origin/main -- --config jest.ci.config.js
```

`init` writes `impact.json` with `"tool": "jest"` or `"vitest"`, the single module `"."`, and
`sources`: `src` plus every top-level directory that holds a test file. Review the source
roots and commit the file; there is no build fingerprint and nothing to `refresh` unless
the roots change. Each changed path decides the selection:

| Changed path | Selects |
|---|---|
| Ignored (`ignore` globs; the defaults never hide files in source roots) | nothing |
| Outside the source roots (`package.json`, lockfiles, `tsconfig.json`, runner configuration) | every test |
| Deleted, in a `__mocks__` directory, or not a script (`.json`, `.css`, fixtures) | every test |
| `__snapshots__/<test file>.snap` | that test file |
| A script (`.js`, `.jsx`, `.ts`, `.tsx`, `.mjs`, `.cjs`, `.mts`, `.cts`) that a runner-loaded file imports, directly or not | every test |
| Any other script | its related test files: the files that import it, directly or not |

Runner-loaded files are setup files, global setup, test environments, transforms,
resolvers, and module-mapper targets, resolved on every run from `jest --showConfig` or
Vitest's resolved configuration, plus, for Vitest, the files its configuration imports. A script that no test file
reaches selects nothing (`NONE`), as an unreached class does at class level, and `run` then
starts no runner. A failed runner query selects every test.

`run` executes `node_modules/.bin/jest` or `node_modules/.bin/vitest` (`--executable`
overrides it): `jest --runTestsByPath FILES…` or `vitest run FILES…` for a `SUBSET`, the
plain command for `ALL`. Arguments after `--` go to both the queries and the run, so pass
`--config` and project filters there. Environment variables and flags inside
`package.json` scripts (such as `cross-env TZ=…`) are not applied; set them on the CI step.
`--records`, `--ci`, `class_level`, and `generated` are not available for these projects.

Selection assumes tests reach project code only through literal `import`/`require`
specifiers or runner-loaded files. Scripts that tests spawn, workers started by path,
script files read as fixtures, and computed imports are invisible: keep such files out of
the source roots. Type checking is not part of the selected run; keep `tsc` as its own CI
step. `replay --run` works for these projects: it installs each commit's dependencies from
its lockfile (npm, pnpm, Yarn, or Bun) and reads the runner's JSON results.
[`projects/js`](../projects/js) carries both configurations and the oracle
`scenarios.json`; `cargo test --test js -- --ignored` checks both runners against it.

## Replaying real history

Fixtures are small, so JVM startup dominates their timings. `replay` measures a real
project instead: configure it with `sieve init` (and optionally `class_level`/`ignore`),
then replay its recent first-parent commits.

```bash
sieve replay --workspace . --commits 20 --run --output /tmp/replay.json
```

Each commit is replayed in a shared clone: its parent and its tree are each committed
with the workspace's `impact.json` (and Maven adapter), so the compared change is exactly
the commit's. Without `--run`, only selections are recorded. With `--run`, every commit
runs `sieve run --full` and `sieve run --base PARENT` from clean trees with the same build
arguments. The report lists selection modes, test cases, seconds, and **missed
failures**: tests failing in the full run whose failures the selected run did not reproduce.
The command exits with 1 for missed failures or build/report problems that prevent
validation. Pass failure-ignore flags so full runs
collect every failure only during evaluation (for Maven, append
`-- -Dmaven.test.failure.ignore=true`); never add them to production test execution.

JSON remains on stdout unless `--output` writes it to a file. A readable Markdown report
appears on stderr and, with `--output /tmp/replay.json`, in `/tmp/replay.json.md`; build
logs remain in `/tmp/replay.json.logs`. Preview runs without `--run` show selections
without claiming measured savings. Run reports compare full and selected build wall time
overall and by selection mode, excluding skipped or inconclusive pairs. The report also
excludes pairs with differing failures or missing/invalid timing data. It shows failure
evidence and counts of full-suite selection reasons; inspect JSON for each commit's
details. An intentional `ALL` run is not necessarily a fallback.

A successful exit with no missed failures is not enough to adopt. Selected-only failures
can still leave replay's exit code at zero; inspect that count, inconclusive comparisons,
and whether any full run actually failed. Without failing full runs, the history provides
no evidence that selection catches real failures. JSON totals retain all recorded
timings for compatibility; use the Markdown report's usable pairs for savings claims.
Timings cover the compared builds, not checkout, installation, queue time, or final CI
completion. Summed historical test durations in a normal `run` summary are estimates of
serial test time avoided, not this measured wall-clock comparison.

### Pilot checklist

Start with three teams whose JVM suites are expensive and whose typical changes affect
a limited part of the tests. For each repository:

1. Record the native test command, JDK, profiles, caches, required reports, and baseline
   PR feedback time. Time setup and note every manual configuration or build change.
2. Run `init` once in the CI build environment; review the graph and POM changes. For
   single-module services, evaluate `class_level` explicitly. Keep records local with
   `--records`, and retain the existing required full-suite checks.
3. Replay at least 20 representative commits with `--run`, saving reports outside the
   workspace. Review source/test changes separately from docs-only changes. Count `ALL`,
   `MODULES`, `SUBSET`, and `NONE`; inspect full-selection reasons, differing failures,
   failing full runs, and inconclusive comparisons before deciding whether to proceed.
4. During a month of rollout, record selected feedback time and final CI completion
   separately, fallback frequency, discrepancies against the full suite, and minutes
   spent maintaining configuration. Stop relying on selection if a failure is missed.
5. After the month, record whether each team still has Sieve enabled and why. Compare
   measured source-change savings with installation and maintenance effort before
   expanding the rollout.

See [coverage and remaining cases](test-coverage.md) and
[measurements](performance.md) for evidence and limits.

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
