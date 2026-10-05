# Architecture

Sieve is a Rust CLI that decides which JVM tests a change can affect and runs them through
the project's own Maven or Gradle build. When it cannot be sure, it runs the full suite.
This page maps the code; [README.md](README.md) is the overview, [docs/reference.md](docs/reference.md) documents behavior and flags,
[CONTEXT.md](CONTEXT.md) the vocabulary, and [docs/adr](docs/adr) the major decisions.
A browsable version with diagrams lives in [site/guide.html](site/guide.html).

## The three selection paths

| Path | Trigger | Evidence | Granularity | Where |
|---|---|---|---|---|
| Module-level | `select`/`run` with `impact.json` | Git diff + declared module graph | Modules | `src/main.rs` |
| Class-level | `"class_level": true`, mode `MODULES` | Compiled bytecode of the revision | Test classes | `src/classes.rs`, `src/generated.rs` |
| Local mode | `run --records` or `"records": true`, single-module Maven or Gradle | Per-test runtime records from earlier runs | Test classes | `src/records.rs`, `agent/` |

Module selection is the default, including single-module projects. Class selection is
always opt-in. A configured `run` without a base selects `ALL` unless local mode was
explicitly enabled. `run --records` can use defaults without `impact.json` on a
single-module Maven project.

Every path produces a `Selection` (`src/main.rs`) with one of four modes:

- `ALL`: every module and test. Chosen for a full request, a build-input change, an
  unclassified path, a stale build fingerprint, or unavailable Git history.
- `MODULES`: changed modules plus their transitive dependents.
- `SUBSET`: named test classes inside the selected modules (class-level and local mode).
- `NONE`: nothing to test. Module-level `NONE` runs only `clean`; class-level `NONE` still
  compiles and verifies the selected modules.

Uncertainty always widens the selection; it never narrows it. Invalid configuration and
build failures fail the command instead of falling back.

## Static path (CI and multi-module projects)

```
sieve run --base REV
  │
  ├─ Config::read            impact.json: tool, module graph, ignore, class_level, generated
  ├─ changed_paths           merge-base with REV; committed, staged, unstaged, untracked paths
  ├─ fingerprint::build_inputs   hash of POMs, Gradle scripts, .mvn/, gradle/, buildSrc …
  │                          mismatch with impact.json → ALL ("run sieve refresh")
  ├─ Config::select          ignore globs → skip; <module>/src/** → module; anything else → ALL;
  │                          then close over reverse dependency edges → MODULES or NONE
  ├─ [class_level && MODULES]
  │    ├─ compile_args       Maven: clean test-compile -pl … -am
  │    │                     Gradle: clean :m:impactCompile :impactClasses
  │    ├─ generated::Base    (optional) merge-base worktree runs generate-test-sources,
  │    │                     diffed generated sources replace the declared inputs
  │    ├─ classes::load      parse every module's class files (incl. unselected modules)
  │    └─ Selection::refine  classes::affected → SUBSET / NONE, or keep MODULES on fallback
  └─ build_args              Maven: clean verify -pl … -am -Dimpact.skip.<m>=…
                                    [+ surefire/failsafe excludesFile]
                             Gradle: clean :m:check -Pimpact.modules=… [-Pimpact.testsFile=…]
```

`select` stops after `Config::select` and prints the JSON; it never compiles, so it stays
module-level. `run` writes `--output` before the build starts and again after refinement,
removes stale Surefire/Failsafe reports outside the reactor, and returns the build's exit
status.

### Class-level analysis (`src/classes.rs`)

`parse` reads a class file's constant pool, descriptors, generic signatures, annotations,
supertypes, dotted string constants, and Kotlin inline-function source maps. `affected`
walks reverse edges from the classes compiled from changed sources:

- a changed type affects its subtypes; a changed implementation affects callers of its
  supertypes (DI, `ServiceLoader`), but not sibling implementations;
- a changed non-private `static final` constant affects every class whose source names it,
  and every class without a findable source;
- a reached DI component (Spring, Jakarta/CDI, Micronaut, Quarkus, JPA, JAX-RS) selects
  every context test, narrowed by Spring Boot slice rules (`@WebMvcTest`, data slices …).

Unmappable changes (deleted sources, `package-info.java`, non-JVM files under `src/`,
unreadable classes) return `Impact::Fallback`, which keeps the module selection.

### Build adapters

- **Maven** (`src/setup.rs`): `init` reads effective models and adds per-module
  `impact.skip.<module>` properties wired to Surefire/Failsafe `skipTests`, so `-am`
  dependencies compile without running their tests. `refresh` rewrites the graph and
  fingerprint and preserves extra declared edges.
- **Gradle** (`src/gradle.init.gradle`): an init script passed with `--init-script`.
  Its `impactInit` task reports the project graph for `init`; it disables `Test` tasks outside
  `impact.modules`, filters tests to `impact.testsFile`, and adds `impactCompile` and
  `impactClasses` for class-level runs. It is configuration-cache safe and requires
  Gradle 7.6.3+.

## Local mode (developer machines)

ADR 0001 adds runtime evidence; ADR 0002 moves selection into the test JVM. One Maven
build runs; the agent asks the binary what to drop and reports what ran.

```
sieve run --records                         (records::run)
  ├─ workspace execution lock
  ├─ key = hash(source contents, build inputs, invocation, env, version, agent jars)
  │    unchanged since last passing run → NONE without starting Maven
  ├─ agent_jar → extract embedded jars to SIEVE_CACHE_DIR, verify bytes
  ├─ Maven: mvn [clean] verify   JDK_JAVA_OPTIONS=-javaagent:sieve-agent.jar=<options>
  └─ Gradle: gradle --init-script sieve.init.gradle -Pimpact.agent=<option> impactTests
        │     (daemon kept, never clean; the init script adds the agent to Test tasks only)
        test JVM ─ Boot (Java 8 entry) → Agent (Java 17+, else inactive)
        │   Transformer   method-entry probes on project classes (output dirs or packaged jars
        │                 in the workspace), file probes on JDK file APIs
        │   Probe/Bucket  bootstrap-loaded sinks: hits per test class / context startup / rest
        │   Filter        JUnit PostDiscoveryFilter ── exec ──▶ sieve decide  → classes to drop
        │   Listener      attributes hits to the running top-level class, records outcomes
        │   spring/*      ContextHook, ContextUse, BootRun: context startup counts for its users
        └── State at JVM exit ── exec ──▶ sieve record  → .sieve/records/<Test>.json
```

`decide` drops a test class when its record passed and nothing it depends on changed: the
test class, JDK, build inputs, invocation, JVM properties and arguments, declared environment inputs
(`record_env`), executed method bodies, read/listed files, class shapes of used classes
and their hierarchy, Spring wiring of used or loaded components, or classes named by string
constants. Constructing a class is not using it (ADR 0004): plain constructors and static
initializers, which only store into the class's own fields, count only for tests that ran
other code of the class. Wiring is compared per part (declaration, constructors, annotated
members, request mappings); configuration files per key (`settings.rs`, through the
project classes that name a key); migration listings per appended file. When such a
narrowed change could stop a context from starting and no full context test runs or has
passed since, one runs as a startup check. A build-input edit is narrowed when the test
class path keeps the same artifacts (`classpaths/`, slots by file name without version) and
only jar contents changed: library methods carry a per-jar probe and `ZipFile` entry reads
report their jar, so a bump reruns the tests that ran or read the jar. Without a record, `--base` enables the static fallback only after a passing
wrapper run with the same invocation (`statically_unreached`). `bytecode.rs` provides the change-insensitive digests: FNV-1a
over method bodies and class shapes with constant-pool references resolved and debug
attributes ignored.

The unchanged-run shortcut hashes source contents as well as metadata. Content edits
that preserve modification times and failed/interrupted previous runs force a clean
build, so Maven cannot test stale classes.
Packaging follows native `verify`; `reuse`, `jgitver`, `mvnd`, and `repackage` are opt-in
speed-ups through `--with`.

State in `.sieve/` (self-ignoring, survives `mvn clean`): `records/`, `snapshots/` of
class shapes, `settings/` with the keys of recorded configuration files and the names of
recorded migration directories, `run/` decisions and summaries, `env-run/` for plain Maven callbacks,
`execution.lock`, its separate `session` token, the record-store `lock`, and the catalog journal. The execution lock
serializes local CLI runs through build and reporting. Plain Maven callbacks run every
test and write no records while another wrapper owns that lock. External Maven or IDE
builds can still modify native outputs; output fingerprints invalidate the shortcut and
class files rewritten during tests prevent recording.

Runtime records assume observed paths cover the test's behavior and that tests are
independent. Undeclared environment inputs, external services, floating image tags, and
dependency jars changed without a build-input change still need full-suite validation.

The agent is compiled by `build.rs` against `agent/stubs` (JUnit Platform and Spring API
stubs), in two layers: `Boot` for Java 8, everything else for Java 17. `Transformer` uses
a vendored ASM relocated to `sieve.agent.asm` (`agent/asm`, ADR 0003). Both jars are embedded with `include_bytes!`; building with
`--no-default-features` omits them and local mode.

## Validation tooling

Selection logic never reads validation expectations.

| Command | Module | Purpose |
|---|---|---|
| `fixtures` | `src/fixtures.rs` | Prepare/mutate `projects/maven` and `projects/gradle`, run builds, compare JUnit XML with `scenarios.json`, benchmark |
| `replay` | `src/replay.rs` | Replay first-parent commits: selection per commit, or with `--run` selected vs full, reporting missed failures; `--walk` for local mode |
| `catalog` | `src/catalog.rs` | Edit catalog timings, `--plant` bugs into changed method bodies, `--levers` speed-up comparison |
| `classify` | `src/catalog.rs` | Weight edit kinds by recent history |
| — | `src/timing.rs` | Timed builds with phase breakdown from timestamped output |

`SIEVE_BROKEN_SELECTOR=1` makes both selectors drop everything; planted-bug runs must
catch it.

## Repository layout

| Path | Contents |
|---|---|
| `src/` | CLI (see table below), Gradle init script |
| `agent/` | Java agent sources, bootstrap probe, compile-only API stubs, service registrations |
| `build.rs` | Compiles and embeds the agent |
| `tests/` | Integration tests: `cli`, `setup`, `native`, `oracle`, `safety`, `records`, `replay`, `workflows` |
| `projects/` | Fixtures: `maven`/`gradle` (3-module parity pair), `single-*`, `records`, `records-reuse`, `containers` |
| `samples/` | `bookstore` (class-level/local demo), `webshop` (5 Spring WebFlux services + system tests), `selective-performance` (module/class timing comparison) |
| `scenarios.json` | Independent oracle for fixture mutations |
| `.github/` | CI (`ci.yml`), compatibility matrix, weekly fixtures, reusable Java build action |
| `docs/`, `issues/` | ADRs, measurements, coverage notes, local-mode roadmap |
| `site/` | Landing page and architecture guide |

| Source | Responsibility |
|---|---|
| `main.rs` | Argument parsing, `Config`, `Selection`, Git diff, module selection, build commands |
| `setup.rs` | `init`/`refresh`, build tool detection, Maven POM adapter, Gradle script extraction |
| `fingerprint.rs` | Hash of conventional build inputs |
| `classes.rs` | Class-file parser and class-level impact graph |
| `generated.rs` | Generated-source comparison against the merge base |
| `records.rs` | Local-mode `run`, `decide`, `record`, `env`, agent extraction, speed-ups |
| `bytecode.rs` | Method-body and class-shape digests, plain constructors, wiring parts, request mappings |
| `settings.rs` | Configuration keys and their project readers, inert Flyway migrations |
| `reports.rs` | Native JUnit XML parsing shared by execution and validation |
| `fixtures.rs`, `replay.rs`, `catalog.rs`, `timing.rs` | Validation and measurement |

## Invariants worth keeping

- Unknown or unclassified change → `ALL`; analysis error → the wider selection.
- Build and test failures propagate; only fixture mutation runs ignore test failures.
- `select` output and `--output` are written before tests run, so CI can publish them.
- Fixture expectations (`scenarios.json`) stay independent of selector code.
- Agent probes never change observed behavior; a possibly unrecorded read discards the
  run's records instead of recording partial evidence.
