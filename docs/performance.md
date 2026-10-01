# Preliminary performance measurements for this repository

Measured 2026-09-21 on macOS ARM64, Java 17.0.20.1, Maven 4.0.0-rc-6 and
Gradle 8.13. The source checkout is based on
`72690f0c0529fdc7579a8168a79c6d93629fa542` plus this implementation's working-tree
changes. Each comparison starts from the same copied fixture and applies the named
mutation from `scenarios.json`; its Git base is the isolated baseline commit. Raw samples, build versions, source
status, selections, execution inventories, and failures are in
[performance-results.json](performance-results.json).

## Local build timing

Seconds: **median (minimum–maximum)**. Each condition has one warmup and five
measured runs. Order alternates between native/full/selected and selected/full/native.
These are preliminary developer-machine measurements. JVM builds ran sequentially,
but short Rust compilation/tests and editing overlapped some samples. They are
useful to expose overhead and large differences, not to establish small speedups.
An isolated rerun is still required for a publishable performance claim.

| Tool | Change | Native full | Sieve full | Sieve selected |
| --- | --- | ---: | ---: | ---: |
| gradle | docs-only | 6.61 (6.32–7.21) | 6.34 (6.23–7.15) | 2.73 (2.62–3.17) |
| gradle | gateway-implementation | 6.95 (6.54–7.08) | 7.08 (6.15–7.39) | 7.05 (6.17–8.47) |
| gradle | tax-transitive | 7.04 (6.47–8.08) | 7.09 (6.53–7.72) | 7.01 (6.70–7.43) |
| maven | docs-only | 4.56 (4.33–5.93) | 6.15 (4.49–8.41) | 1.86 (1.84–4.16) |
| maven | gateway-implementation | 4.58 (4.31–5.13) | 4.32 (4.24–4.51) | 4.01 (3.85–5.70) |
| maven | tax-transitive | 4.51 (4.36–4.68) | 4.53 (4.27–5.06) | 4.42 (4.00–4.89) |

- `gateway-implementation`: checkout only, 5 invocations versus 17 native.
- `tax-transitive`: pricing and checkout, 12 invocations versus 17 native.
- `docs-only`: `NONE`, zero test invocations; native `clean` still runs.

Every timed run passed its independent fixture oracle. Mutation runs deliberately
contain assertion failures; fixture-only failure-ignore flags collect the entire
expected failure set. A missed failure, wrong inventory, unexpected error, or wrong
invocation count aborts the benchmark and removes its summary.

Dependencies were warm and shared; workspaces were fresh. Native and Sieve runs use
the same clean/verify or clean/check lifecycle. Timing includes the child process
and report parsing, but excludes fixture copying, Git setup, Sieve compilation,
checkout, and artifact uploads. These short source-change builds offer little
reliable saving; ranges and startup cost matter. Docs-only changes avoid more work.
This is not evidence of a general CI speedup.

## Real history replay: apache/commons-text

Measured 2026-09-24 on macOS ARM64, Java 17.0.20.1, Maven 3.9.16, with
`sieve replay --commits 30 --run` over the 30 first-parent commits ending at
`8b5fc735`. commons-text is one Maven module with 103 test classes and about 1,900
test cases; its full `verify` takes about 16 s. Configuration: `sieve init`, then
`"class_level": true` and an adopter's ignore list (`*.md`, `*.txt`, `.asf.yaml`,
`.github/**`, `src/changes/**`, `src/site/**`). Both runs of every commit skip the same
report plugins (RAT, japicmp, Checkstyle, SpotBugs, PMD, JaCoCo, Javadoc, CycloneDX,
SPDX) and ignore test failures to collect them all. Per-commit data is in
[replay-commons-text.json](replay-commons-text.json).

| Mode | Commits | Changes | Build time full → Sieve |
| --- | ---: | --- | ---: |
| NONE | 15 | CI action bumps, release notes, site, `.asf.yaml` | 256 s → 13 s |
| SUBSET | 7 | Java sources and tests (1–64 of 103 classes) | 118 s → 73 s |
| ALL | 8 | POM, dependency and plugin changes | 138 s → 136 s |
| **Total** | **30** | | **511 s → 222 s (−57%)** |

Executed test cases fell from 57,743 to 19,822 (−66%). Timings use a monotonic clock
and exclude the laptop's sleep periods; JUnit's wall-clock times in the logs do not.

- **NONE carries most of the saving.** It still starts Maven for `clean` (about 1 s).
  Eight commits touch only `.github/`, six only
  release notes or site sources. Ignoring `.github/**` is an adopter decision: a
  workflow change can alter the JDK or test flags. Before ignore globs applied inside
  module source folders, the six `src/changes`/`src/site` commits ran every test.
- **SUBSET saves less on a fast suite.** The compile step costs a second Maven start, so
  a change reached by 37 classes took 15 s against 16 s. Two `Javadoc` commits still
  selected 39 and 64 classes: selection follows changed sources, not changed bytecode.
- **ALL costs the same as the full suite**; the selection overhead is within noise.
- **No failure was missed, but none could be:** no full run in this window failed. The
  replay shows savings on real history; safety evidence still comes from the fixture
  mutations. A window containing failing commits is needed for real-failure evidence.

One repository and one machine; a slower suite or per-class setup costs (containers,
application contexts) would raise the SUBSET saving, and a multi-module replay remains
open.

## Actual GitHub Actions cost

[Run 35655467667](https://github.com/preacherxp/sieve/actions/runs/35655467667)
measured the existing workflow at `72690f0`, before this implementation. Its base,
`8a4adce`, differs only in workflow and documentation files. The workflow changes
force `ALL`; the selected stage defers testing to the required full stage.

- 23 completed jobs; **2,003 aggregate runner seconds** (33m23s).
- **387 seconds** (6m27s) from earliest job start to latest job completion.
  Queue time before the first job is excluded.
- Selected jobs: Maven 28s, Gradle 43s; Rust build/setup alone took 17s and 16s.
- Full jobs: Maven 204s, Gradle 314s, including 157s/262s of mutation verification.
- Two artifact uploads failed with HTTP 403. The test steps passed. This is a
  completed failed workflow, not a successful validation of the new changes.

The full stage remains required after selected feedback. That design validates
Sieve's correctness and adds CI work; selected feedback time and final workflow
completion are different measurements. Added regression checks have not yet run
on GitHub, so their new aggregate CI cost is not measured here.

## Reproduce

```bash
cargo build --locked --release
for scenario in gateway-implementation tax-transitive docs-only; do
  target/release/sieve fixtures benchmark --tool both --scenario "$scenario" \
    --runs 5 --output validation-results/timing
done
GITHUB_REPOSITORY=preacherxp/sieve bash scripts/ci-cost.sh 35655467667
```

Use `--maven` and `--gradle` for explicit executables and the same JDK as CI.
Cold dependency/image caches, native task-cache hits, larger module graphs,
service timings, and a frequency-weighted replay of real commits remain separate
experiments. Docker was unavailable locally; no container-time saving is claimed.

## Single-module setup: reproduce no gain, then select test classes

Reviewed 2026-09-30: module selection cannot skip any tests when a project has only
one module. The bytecode selector and native test filters already support skipping
unrelated test classes. Set `"class_level": true` for the measured workload below;
`init` keeps it opt-in because cheap suites can become slower. `refresh` preserves
the choice. The sample reuses the existing selector rather than introducing a second
selection algorithm.

[`samples/selective-performance`](../samples/selective-performance/README.md) has a
fast price test and an unrelated test with a configurable delay standing in for
expensive application/container setup. Changing `Price` selects its only module,
so module selection executes both tests and does no less work than native full.
Class selection executes only `PriceTest`.

Measured on macOS ARM64, Java 17.0.20.1 and Maven 3.9.16, using the release build
at `48209306b4301bbfac09acaac9293b9dd9dd917a` plus working-tree changes. Each condition
has one warmup and five samples, with alternating execution order and warm shared
dependencies. JVM builds ran sequentially. Seconds are **median (minimum–maximum)**:

| Unrelated setup delay | Native full | Module selection | Class selection |
| --- | ---: | ---: | ---: |
| 6 seconds | 7.412 (7.373–7.453) | 7.457 (7.451–7.590) | 2.306 (2.179–2.325) |
| 0 seconds | 1.377 (1.364–1.400) | 1.426 (1.411–1.468) | 2.206 (2.186–2.223) |

The six-second case goes from **no useful module-level saving to 69% lower wall
time** with class selection. Both full and module runs execute two test cases;
class selection executes one. The zero-delay case is **60% slower than full**
with class selection: the second build-tool startup costs more than skipping a
trivial test saves. Leave `"class_level"` off for cheap suites. Class selection
does not remove the two-build overhead or guarantee a performance gain.

Every sample's executed test cases and failures are verified from Maven XML,
independently of the selection report. In both workloads, an unavailable Git base
fell back to `ALL` and executed both tests. A deliberately incorrect price calculation
failed the same `PriceTest` in native full and selected runs; both returned a failure
exit code. The standard Rust suite (69 tests), formatting, Clippy, and the real
Maven/Gradle class-selection integration check passed. Gradle's wrapper needed access
to the existing build cache outside the sandbox. No GitHub Actions execution was
performed and no CI workflow was changed.

Raw samples, test inventories, failure checks and tool versions are in
[selective-performance-results.json](selective-performance-results.json). Logs and
selection reports remain in the ignored `validation-results/selective-performance/`
and `validation-results/selective-performance-zero/` directories. These synthetic
measurements include Sieve, compilation, JVM startup and tests; they exclude setup,
checkout, installation and later CI steps. No real service startup or cold-cache
saving is claimed.

```bash
cargo build --release --locked
python3 scripts/benchmark-selective-performance.py --maven /path/to/mvn
python3 scripts/benchmark-selective-performance.py --maven /path/to/mvn \
  --delay-ms 0 --output validation-results/selective-performance-zero
```

## Local mode on a slow single-module service: commit walks

Measured 2026-10-01 on macOS ARM64 (12 cores), JDK 24.0.1, Maven 3.9.11 and Gradle 8.14
(daemon), Docker 29. The benchmark is a purpose-built Spring Boot 3.5 teleconsultation
service (Java 21 bytecode with Kotlin reporting, PostgreSQL and Kafka through
Testcontainers, 233 tests in 62 classes) whose 35-commit history was written and frozen
before Sieve ran on it. Its integration tests extend a base class with `@DirtiesContext`,
so each context test class starts Spring, PostgreSQL and Kafka. The same sources build
with Maven and Gradle. A native full build takes 103–232 s per commit. Raw data:
[walk-bench-clinic-maven.json](walk-bench-clinic-maven.json),
[walk-bench-clinic-gradle.json](walk-bench-clinic-gradle.json).

`sieve replay --walk --plant --commits 20` applied the last 20 commits in order to one
working tree, ran `sieve run --base PARENT` against a native build of the same tests
(Maven `verify`; Gradle's `Test` tasks without linters), and planted one bug per commit
with changed main code. Seconds, totals per commit kind:

| Commits | Maven native | Maven Sieve | Gradle native | Gradle Sieve |
| --- | ---: | ---: | ---: | ---: |
| 4 narrow (test-only, docs, CI, a failing test rerun) | 623 | 80 (−87%) | 663 | 89 (−87%) |
| 14 wide (component, configuration, migration, wiring) | 2,281 | 2,071 (−9%) | 2,364 | 2,305 (−2%) |
| 2 build inputs (POM, Kotlin version) | 311 | 324 (+4%) | 293 | 305 (+4%) |
| **20 total** | **3,215** | **2,475 (−23%)** | **3,319** | **2,698 (−19%)** |

- **Edits the records can narrow pay off most.** A test change ran 1–3 tests in 7–10 s
  instead of 110 s. A bug planted in a method body ran 1–13 tests in 2–21 s, and failed.
- **Spring wiring limits the rest.** `application.yml`, a Flyway migration, a constructor,
  or a Spring Data repository method is read or wired by every context, so every context
  test reruns; with `@DirtiesContext` each of them restarts its containers. Removing
  `@DirtiesContext` at HEAD cut the native Maven build from 135 s to 99 s (−27%) and
  broke one test that read another test's Kafka events: test isolation is the other lever.
- **No real failure was missed.** Maven reported 2 missed failures, both from a test
  whose result depended on test order, before the commit that fixed it. Gradle reported
  4: two were PostgreSQL connection drops at container start in the native runs, and two
  came from a planted bug that did not compile, which the walk did not yet recognize on
  Gradle (fixed since). Planted bugs were detected in 11 Maven and 10 Gradle commits.
- Earlier planted-bug runs selected every test: planted runs added a failure-ignore flag
  that normal runs lacked, which changed the records' invocation context. All runs of a
  walk or catalog now share the flag; walks and catalogs from before 2026-10-01 should be
  repeated.

One benchmark and one machine, built by the same author as the tool; a service with
shared contexts or fewer wiring edits will show different shares.
