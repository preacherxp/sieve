# Sieve: Java and Kotlin test impact

A Rust CLI that runs tests in changed JVM modules and their transitive dependents.
It supports Java, Kotlin/JVM, and mixed projects using Maven or Gradle. The selector,
fixture manager, validation oracle, and tests are Rust; an optional test-JVM agent for
local mode is Java.

Selection is conservative and builds only the selected modules and what they depend on.
Setup uses module selection by default; projects can opt into class-level selection
from compiled bytecode after measuring their test costs.
Single-module Maven projects can also opt into local mode, which keeps
per-test runtime records and skips tests whose records show no change (see
[Local mode](#local-mode-test-records)). Build configuration changes or uncertain Git
history trigger the full suite.

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
need manual graph review. Automatic setup collects declared inter-project edges, and also
edges the build itself creates: for Maven, a sibling module used as a build plugin, a plugin
dependency, an annotation processor path, or an unpacked artifact, and a sibling directory
named in the effective build configuration (such as a shared OpenAPI specification); for
Gradle, a source set directory inside another project.

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
replaces it. The default never hides module sources, such as those of a module named
`docs`. The samples also ignore the repository's `VALIDATION.md`. Other changes
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

### Class-level selection

Module-level selection runs every test of a selected module, and a single-module
project has only one. Add `"class_level": true` to narrow a module selection to test
classes. It is opt-in because the extra compilation invocation can outweigh the time
saved in cheap suites; `refresh` preserves that choice.
`run` first compiles the selected modules
(`clean test-compile -pl <selected> -am` on Maven, `clean :<module>:impactCompile` on
Gradle), then reads the class files of every module:

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

The second build skips `clean`, and Maven also skips recompiling the main classes it just
compiled (`-Dmaven.main.skip=true`); plugins that post-process classes in place run again
on the already processed output. Maven receives `-Dsurefire.excludesFile` and
`-Dfailsafe.excludesFile` listing the unselected test classes, so POM includes and the
unit/integration split stay in effect; the file also repeats Surefire's default
`**/*$*` exclude, which an excludes file otherwise drops. Gradle reads the selected
classes from a file (`-Pimpact.testsFile`) and filters every `Test` task to them and
their nested classes. `select` stays module-level because it does not compile;
`--output` receives the refined decision. `refresh` preserves the flag.

Static analysis still cannot see classes named in resources outside `src/`, reflection
built from non-constant strings, context tests whose composed annotation lives in a
library jar, or scanning by frameworks not listed above. A Spring application
change usually reaches a component, so context tests are selected together with the
unit tests that reach it. Keep full-suite runs on the default branch.
[`samples/bookstore`](samples/bookstore/README.md) demonstrates the savings: an edit
selects 1–3 of its 10 test classes. [`samples/webshop`](samples/webshop/README.md) applies
both levels to five Spring WebFlux services and an end-to-end module, and replays a
history of breaking and fixing commits against the full suite.
[`samples/selective-performance`](samples/selective-performance/README.md) reproduces
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
Alternatively, commit `"records": true` in `impact.json` to enable it for subsequent
`run` commands, including `--base` and `--full`. Without either opt-in, selection is
static and requires `impact.json`. For a configured local project:

```bash
sieve run --base origin/main   # static fallback after a green run with the same invocation
sieve run --full               # run everything, still recording
```

`sieve run` starts Maven once with an incremental `verify`. It
adds `clean` only when stale output is possible (a file deleted
or renamed since the last run, a build input edit, a `generated` input edit, no
earlier run, or a failed/interrupted previous run). A content edit that preserves file metadata also cleans to prevent stale
compiler output. When nothing changed since the last passing run, including the command,
environment, and Sieve version, it reports `NONE` without starting Maven. Build arguments
after `--` are appended; if they name goals or phases, they replace `verify`.
Concurrent wrapper runs in one workspace are serialized through an OS file lock.

On Gradle, `sieve run` starts one build on the Gradle daemon and never adds `clean`:
Gradle's incremental compilation removes the output of deleted sources itself. By default
it runs every `Test` task (`impactTests`, added by Sieve's init script) without linters or
packaging; task names after `--` replace it, such as `sieve run -- check`. The init script
adds the agent to the `Test` tasks' JVMs only, never to the Gradle or Kotlin daemons.
Gradle's own `--tests` filter always runs the named tests, through the `test` task when
no task is named.

The embedded agent reaches every test JVM through `JDK_JAVA_OPTIONS`, so POM `argLine`
settings (including JaCoCo's) are kept; the agent ignores Maven's own JVM. At JUnit
Platform discovery it asks `sieve decide` which test classes to drop, and after the run
it hands `sieve record` what each top-level test class executed: every project method,
and every workspace file it opened, looked up, or listed through the JDK's file APIs
(including class-path resource lookups, which also record resources that do not exist
yet, and directory listings). Test property files named by `@TestPropertySource` count
as read while the context starts. Work done while a Spring
test context or Spring Boot application starts counts for every test class that uses the
context, including classes that ran after it was cached. Work that outlives a test class,
such as a message it sent and did not await, counts for that class. Records live in
`.sieve/`, which ignores itself in Git and survives `mvn clean`. They accumulate across a
test class's passing runs until the class itself changes.

A test class is dropped when its last run passed and none of the following changed since:

- the test class, its nested classes, the JDK, or build inputs (including parent POMs
  outside the workspace);
- Maven or Gradle invocation arguments, stable JVM system properties, or declared environment inputs
  (`"record_env": ["SERVICE_MODE"]` in `impact.json`);
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
`-Dit.test`, Gradle's `--tests`) always run. A JVM that runs test classes in parallel keeps no records and
drops nothing; so does a run in which a probe failed, no project class was instrumented,
or class files changed while the tests ran. A class that failed in the build reports, or
ran without the agent reporting it in a failed build, counts as failed, and a build whose
test failures were ignored does not count as passing. `--output` gives a reason for
every test class, and lists dropped ones under `skipped`; build reports omit them.

Plain Maven can use test records through the agent option:

```bash
JDK_JAVA_OPTIONS="$(sieve env --workspace .)" mvn verify
```

Plain Maven runs unrecorded tests; it has no known previous wrapper invocation for
static fallback. Its agent files are isolated from wrapper run state.

[docs/local-mode-adoption.md](docs/local-mode-adoption.md) walks a service through
adoption, measurement, and the test-setup changes that pay off most.

Records assume that every path a test can take has shown up in one of its passing runs
since the test class last changed, and that tests do not depend on what earlier tests
left behind. Not tracked: undeclared environment variables, external services,
floating Docker image tags, dependency jars changed without a POM change, and lazily
created beans, whose startup counts only for the test class that first used them. JVMs
older than Java 17 (back to Java 8) load the agent but keep it inactive, so every test
runs; so do class files newer than the vendored ASM can read. Surefire
with `useSystemClassLoader=false` is not supported. Multi-module projects and CI keep
today's module- and class-level behavior. Plain Gradle runs without Sieve get no records.

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

Local speed-ups are opt-in with `--with NAME[,NAME]`; `--without` overrides them.
`--output` lists each one's state under `speedups`:

- `reuse`: `TESTCONTAINERS_REUSE_ENABLE=true`, so containers declared `withReuse(true)`
  survive between runs.
- `jgitver`: `-Djgitver.skip=true` when `.mvn/extensions.xml` loads jgitver. Filtered
  resources that embed the version change, and so rerun their readers.
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
