# Sieve test coverage plan

This checklist covers the current Java and Kotlin/JVM module selector, its Maven
and Gradle adapters, and the evidence needed to claim faster CI. Coverage inventory:
2026-09-21. Expectations describe what a check must establish, not a claim that
every case already passes.

Add a fixture when it exercises a different dependency edge, input location, build
behavior, or failure mode. Java syntax features alone do not change this selector:
it selects whole modules and their transitive dependents.

## Priorities and status

- **P0:** Required to trust selection, failure propagation, or installation.
- **P1:** Required to evaluate real CI savings and common consumer builds.
- **P2:** Extend when a consumer needs the named feature or environment.
- **Existing:** A focused automated case exists; this does not certify its latest run.
- **Partial:** Some relevant checks exist, but the stated acceptance is incomplete.
- **Missing:** No focused automated check was found.

Use the existing Rust tests for selection and Git cases, the fixture verifier for
native execution, and the existing workflows for CI checks. A new test framework
is unnecessary. Maven and Gradle fixtures must stay equivalent where both tools
support the case.

## Acceptance rules for every mutation

1. Start full and selective execution from equivalent isolated workspaces at the
   same revision. A passing baseline must execute its complete test inventory.
2. Define changed inputs, dependency edges, required tests, expected failing
   classes, and expected selection mode before invoking the selector.
3. The full run must observe the declared failures. The selective run must include
   every required test and observe those failures too. A compilation failure is
   a build failure, not successful detection of an expected assertion failure.
4. Check native XML reports: executed classes, invocation counts, failures, errors,
   and skipped cases. Do not accept successful exit status with missing tests.
5. Assert exact selected modules when module selection is the contract. Extra
   classes within those modules are expected. Keep optional class-level precision
   checks separate from correctness checks.
6. `ALL` runs the full suite; `NONE` runs only `clean` under the current runner
   contract. Ordinary build commands must continue to run all tests.
7. Production `sieve run` must fail on test/build failure. Failure-ignore flags
   belong only to deliberate mutation verification so all modules can finish.

Keep the oracle independent in [scenarios.json](../scenarios.json). Never derive
the required test set or expected failures from Sieve's selection. New fixtures
may require extending the verifier's fixed module/report inventory; do not weaken
its assertions to accommodate them.

## 1. Preserve the existing application mutations

All 20 mutations below already have fixture definitions. Their native full and
selective checks must run for both build tools. `pricing + checkout` means tests
in both modules; it does not mean only classes directly calling the changed code.

| Case | Priority | Expected selection and behavior |
| --- | --- | --- |
| `tax-transitive` | P0 | Pricing + checkout; catch all seven declared failing classes after a tax change. |
| `calculator` | P0 | Pricing + checkout; catch all six declared failing classes after a shared calculation change. |
| `unrelated` | P0 | Pricing + checkout; currency failure is detected and runtime tests are excluded. |
| `unused` | P0 | Pricing + checkout even though no test fails; record conservative extra execution. |
| `gateway-implementation` | P0 | Checkout; interface implementation changes reach the declared consumer tests. |
| `inherited-fixture` | P0 | Checkout; changes to a shared test superclass rerun its module. |
| `changed-test` | P0 | Pricing + checkout; a changed test is included and its deliberate failure is observed. |
| `new-test` | P0 | Pricing + checkout; the added class executes even though it was absent from the baseline inventory. |
| `reflection` | P0 | Runtime; a reflectively reached implementation failure is observed. |
| `async` | P0 | Runtime; a failure from code running on a worker thread is observed. |
| `spring-bean` | P0 | Runtime; both injection and ServiceLoader failures are observed. |
| `spring-wiring` | P0 | Runtime; changing the configured bean reaches the integration test. |
| `resource` | P0 | Runtime; a classpath resource mutation reaches its consuming test. |
| `service-provider` | P0 | Runtime; changing the service registration reaches its consuming test. |
| `delete-class` | P0 | Checkout; deletion plus the compilable consumer edit produces the declared failure. This does not prove isolated deletion handling. |
| `docs-only` | P0 | `NONE`; native execution produces no test reports after cleanup. |
| `dependency-change` | P0 | `ALL`, even though the manifest's minimum required tests are only in runtime. |
| `kotlin-source` | P0 | Pricing + checkout; the Kotlin production change is detected. |
| `java-record` | P1 | Pricing + checkout; the record factory's contract failure is observed. |
| `kotlin-sealed` | P1 | Pricing + checkout; the sealed result formatter's failure is observed. |

Current counts, for detecting inventory drift:

| Execution | Classes | Invocations |
| --- | ---: | ---: |
| Baseline full suite | 15 | 17 |
| Pricing + checkout | 10 | 12 |
| Checkout only | 4 | 5 |
| Runtime only | 5 | 5 |
| `new-test`, full suite | 16 | 18 |
| `new-test`, selected | 11 | 13 |

Update every affected inventory assertion when adding tests. In particular,
`installs_build_adapters` must agree with the tax scenario: 12 invocations and
seven failing classes. An ignored integration test is not exercised by the default
`cargo test` command.

## 2. Git changes and input classification

Evidence: [CLI tests](../tests/cli.rs) and [selection implementation](../src/main.rs).
Use temporary Git repositories; most of these cases need no JVM build.

| ID | Priority | Case and acceptance | Status |
| --- | --- | --- | --- |
| GIT-01 | P0 | Clean tree against `HEAD` selects `NONE`; committed, staged, unstaged, and untracked source edits select their owners and dependents. | Existing |
| GIT-02 | P0 | Several changed files and modules produce the union of affected modules without duplicates; a commit fully reverted relative to the base selects `NONE`. | Missing |
| GIT-03 | P0 | Rename within a module or move across modules; include both old and new owners and their dependents, for staged and committed moves. | Partial: an unstaged cross-module move exists |
| GIT-04 | P0 | Delete production code, a test, or a resource without another edit selecting the same module. Select the old owner; distinguish an expected test failure from a compilation failure. | Partial: current deletion also edits its consumer |
| GIT-05 | P0 | A diverged target branch uses the merge base; a synthetic PR merge checkout compared with the PR base selects the correct changes. | Partial: diverged branches are covered |
| GIT-06 | P0 | Unknown base, unrelated histories, genuinely shallow history, and unavailable Git each select `ALL` with a useful reason. Invalid project configuration still fails. | Partial: unknown revision is covered |
| GIT-07 | P0 | Root build files, module build files, wrappers, lockfiles, `impact.json`, CI scripts, and unknown tracked/untracked inputs select `ALL`. | Partial: representative build and unknown paths are covered |
| GIT-08 | P0 | Root `README.md`, `VALIDATION.md`, and `docs/` select `NONE`; similarly named files under module `src/` select that module. Mixed docs and source edits retain source selection. | Partial: representative docs and resource paths are covered |
| GIT-09 | P0 | A workspace below the Git root still notices shared and sibling repository changes and selects `ALL`; recognized repository docs retain their exemption. | Missing |
| GIT-10 | P1 | Spaces, Unicode, and newlines in source filenames survive Git's NUL-delimited output; an undecodable filename causes a full fallback. Workspace paths with spaces work. | Missing |
| GIT-11 | P0 | Ignored build outputs and reports outside the repository do not change selection on a repeat run; an unignored output file selects `ALL` with a diagnostic. | Missing |

## 3. Dependency graphs and module ownership

Use flat module directories for these cases so graph tests stay within the current
layout contract. Discover edges through `init` as well as testing the selector with
explicit graphs; a correct traversal cannot compensate for missing discovered edges.

| ID | Priority | Case and acceptance | Status |
| --- | --- | --- | --- |
| DEP-01 | P0 | Single module, two independent modules, and a direct consumer. Select only the changed owner and dependents; a consumer-only edit does not select its provider's tests. | Existing |
| DEP-02 | P0 | A three-hop chain and a diamond. Include every transitive dependent once, exclude unrelated modules, and avoid depending on map iteration order. | Missing: current native graph has only one dependency edge |
| DEP-03 | P0 | Runtime-only project dependency: change a provider reached through reflection or ServiceLoader in another module; discover the edge and execute the consumer's failing test. | Missing: current dynamic-call fixtures stay within one module |
| DEP-04 | P0 | Resource-only dependency between modules: change provider configuration, service registration, or a template; include the consuming module even without a Java import. | Missing |
| DEP-05 | P0 | Test-only dependency or shared test fixtures: change the provider, then independently change only the consumer. Select required tests and still produce prerequisite test artifacts after `clean`. | Partial: shared superclass is only within one module |
| DEP-06 | P0 | Add or remove a module/dependency: the build/configuration edit selects `ALL`; after updating and committing the graph, a subsequent source edit follows the new graph. | Missing |
| DEP-07 | P0 | Leave `impact.json` stale after a dependency change, commit, then change the new provider. Required safety target: detect the mismatch and select `ALL` or fail clearly before trusting a subset. | Missing; current implementation requires manual graph maintenance |
| DEP-08 | P0 | Empty aggregator modules and a root project with its own sources/tests retain ownership and dependency edges during discovery and selection. | Partial: standalone root package is covered |
| DEP-09 | P1 | Duplicate edges and a cycle terminate with a stable selected set. Test traversal directly; native build-tool rejection of an invalid cycle must still fail the run. | Partial: selector unit test covers a cycle |

DEP-07 is a proposed safety improvement, not an implemented guarantee. Until it is
implemented, keep graph review explicit. Arbitrary undeclared runtime dependencies
cannot be assumed detectable; supported fixtures must declare those edges.

## 4. Setup and native build adapters

Evidence: [setup](../src/setup.rs), [Gradle adapter](../src/gradle.init.gradle),
[installer tests](../tests/cli.rs), and [compatibility smoke checks](../scripts/compat-smoke.sh).

| ID | Priority | Case and acceptance | Status |
| --- | --- | --- | --- |
| BUILD-01 | P0 | Install into a clean Maven and Gradle project without the sample's existing skip adapter; discover the same graph and run the selected failures through the generated setup. | Existing |
| BUILD-02 | P0 | Repeat `init`, encounter an existing Maven profile, or fail build-model discovery. Refuse conflicts, preserve existing configuration, and make no edits after failed discovery. Ordinary native commands still run all tests after installation. | Partial |
| BUILD-03 | P0 | Prefer an executable project wrapper over PATH; honor `--executable` for both setup and execution. Missing or non-executable commands fail clearly. | Missing |
| BUILD-04 | P0 | Reject malformed/unknown configuration fields, missing module directories, unknown dependency names, invalid module paths, ambiguous build-tool detection, and duplicate Maven coordinates. Never silently report `NONE`. | Missing: validation code exists without focused cases |
| BUILD-05 | P0 | Reject unsupported nested/custom layouts, composite builds, Android, Kotlin Multiplatform, and Gradle below the supported minimum during automatic setup, with actionable errors and no generated setup. | Missing: guards exist without focused rejection cases |
| BUILD-06 | P0 | Maven parent inheritance, properties, and active profiles produce the correct module graph in the CI environment. Execution receives the applicable profile/property flags; selected tests and unrelated POM configuration are preserved. | Partial: basic effective-POM discovery and XML profile insertion are covered |
| BUILD-07 | P0 | Maven Surefire and Failsafe honor per-module skipping, including inherited/plugin-specific configuration. Skipping provider tests must not remove artifacts needed by selected consumers. | Partial: ordinary unit/integration suites are covered; combine artifact case with DEP-05 |
| BUILD-08 | P0 | Gradle Groovy and Kotlin DSL: single package, direct child modules, and custom `Test` tasks connected to `check`. Every selected test task runs and every excluded test task skips. | Partial: mixed multi-module Groovy and standalone Kotlin DSL are covered |
| BUILD-09 | P0 | Java-only, Kotlin-only, and mixed sources preserve Java-to-Kotlin and Kotlin-to-Java test reachability at module boundaries. Include test resources. | Partial: Kotlin-to-Java, standalone Kotlin, and Java smoke checks exist |
| BUILD-10 | P1 | Forward applicable native arguments after `--` intact. Coverage, packaging, and custom tasks required by a consumer's existing job still execute; validate behavior when selection is `NONE` and those outputs would be absent. | Missing |
| BUILD-11 | P0 | Exercise the declared JDK/Maven/Gradle/Kotlin compatibility combinations with actual `init` and selected execution. Cover both branches of the Gradle project-dependency API adapter. Keep Java 8/11 smoke fixtures separate from Java 17 sample features. | Existing matrix; each run still needs verification |
| BUILD-12 | P2 | A consumer using JUnit 4, JUnit 5, TestNG, parameterized/dynamic tests, or nonstandard suite names retains native test discovery and accurate report accounting. | Partial: JUnit 4/5, parameterized tests, and two methods in one class exist |

Non-`Test` Gradle tasks that launch tests, custom generated-source layouts, and
external dependency substitution need an explicit support decision before adding
positive support claims. Test conservative fallback for recognized build-input
changes; do not imply semantic discovery of these relationships already exists.

## 5. Failures, reports, and the validation oracle

Evidence: [runner](../src/main.rs), [CLI tests](../tests/cli.rs), and
[oracle/report parser](../src/fixtures.rs).

| ID | Priority | Case and acceptance | Status |
| --- | --- | --- | --- |
| RUN-01 | P0 | Passing selected/full builds return success; real unit and integration failures return nonzero. Preserve the selection JSON before launching the build, including failure paths. | Partial: fake build failure and real Kotlin failure exist; complete both native adapters and suites |
| RUN-02 | P0 | Compilation errors, dependency-resolution failures, missing JDK/build tools, and interrupted builds cannot appear as a passing selected run or a successful mutation. | Missing |
| RUN-03 | P0 | Seed stale XML reports, then execute a smaller subset or `NONE`. Old results disappear, and the reported inventory contains only this execution. | Partial: `clean` is always used; add a deliberately dirty-workspace regression |
| RUN-04 | P0 | Missing, empty, truncated, and malformed reports fail verification when tests are expected. Skips, errors, multiple methods, and parameterized invocations are accounted for independently of class count. | Partial: parser tests cover several shapes; missing-report execution needs a focused case |
| RUN-05 | P0 | Inject an omitted affected test, an unexpected extra executed test, a wrong failure set, and wrong invocation counts. The verifier must reject each even when the build exits zero. | Partial: selection omission/payload validation exists; add complete verifier negative cases |
| RUN-06 | P0 | `--full` and omitted `--base` run all tests; contradictory flags, unknown options, and missing option values fail clearly. A base resembling an option is treated only as a revision. | Partial |
| RUN-07 | P0 | `select` does not launch a build. An unwritable selection destination causes a visible error before execution; saved selection matches the actual native filtering arguments. | Partial: selection-before-failure is covered |
| RUN-08 | P0 | Fixture preparation refuses an existing destination; mutations refuse stacking, path traversal, symlink escapes, and unmatched replacement text without partial mutation. | Partial: existing destination, stacking, and traversal cases exist |

For new or deleted tests, keep the post-mutation inventory independent of selector
output. The current inventory helper adds `required` tests to the baseline; a test
deletion case will need explicit removal accounting.

## 6. Container and service integration

The current Kafka and Redis smoke jobs establish service connectivity. They run
Maven directly from [projects/containers](../projects/containers/pom.xml), so they
do not yet establish Sieve selection or avoided container startup.

| ID | Priority | Case and acceptance | Status |
| --- | --- | --- | --- |
| INT-01 | P1 | Baseline Kafka produce/consume and Redis SET/GET succeed using mapped ports, bounded waits, and cleanup on success/failure. | Existing standalone tests |
| INT-02 | P0 | Put a service-backed consumer in a supported module graph. Change a declared provider and assert the consumer's container tests are selected and its deliberate failure is detected. | Missing |
| INT-03 | P1 | Change an independent module. Exclude the service module, execute its selected peers, and prove that no Kafka/Redis container starts. Empty test XML alone is insufficient evidence of avoided startup. | Missing |
| INT-04 | P0 | Change service configuration under module resources versus container image/version configuration in a build file. Select the owning module/dependents in the first case and `ALL` in the second. | Missing |
| INT-05 | P0 | Docker unavailable, image pull failure, service startup timeout, or a failing assertion returns failure with usable logs and cleans up created containers. Distinguish infrastructure failure from a fixture's expected assertion failure. | Partial: successful smoke runs and scoped cleanup exist |
| INT-06 | P1 | Parallel service test jobs and repeated runs do not share ports, topics, keys, or stale state that can change the selection verdict. | Partial: random mapped ports and per-run Kafka topics exist |

Start with the existing Kafka and Redis fixtures. Add databases or other services
only when they expose a different selection/setup issue or a real consumer needs them.

## 7. GitHub Actions behavior

Evidence: [main workflow](../.github/workflows/ci.yml),
[test workflow](../.github/workflows/java-tests.yml), and [integration rules](../AGENTS.md).
Existing workflow branches count as implementation, not focused regression tests.

| ID | Priority | Case and acceptance | Status |
| --- | --- | --- | --- |
| CI-01 | P0 | A pull request retains the default merge checkout, uses the PR base SHA, and fetches enough history. Verify selection using a real or locally constructed merge commit. | Partial: configured |
| CI-02 | P0 | Push selection uses the previous commit when available. Empty/all-zero push SHAs, manual runs, and other events default to full execution. Missing history also falls back to full execution. | Partial: configured |
| CI-03 | P0 | Selection `ALL`, `MODULES`, and `NONE` each reach the intended job path. If this repo defers `ALL` to a separate full stage, prove that stage runs; a consumer job without that stage must execute `ALL` itself. | Partial: configured |
| CI-04 | P0 | Failed selected execution leaves the job failed, preserves available selection/native reports, and still permits required full validation unless cancelled. Missing optional reports do not mask the original error. | Partial: configured |
| CI-05 | P0 | Quote event values via environment variables and arrays; filenames/revisions containing shell punctuation cannot become commands. Run PR code under `pull_request`, with appropriate permissions and checkout credentials. | Partial: workflow conventions exist |
| CI-06 | P0 | A generated report does not become an untracked selector input. A real `MODULES` scenario reaches native execution even when a repository-wide edit makes the ordinary selected job defer `ALL`. | Partial: ignored reports and explicit fixture verification exist |
| CI-07 | P1 | Every intended scenario appears in the executed job inventory; matrix grouping has no missing or duplicated scenario coverage. Report job count, aggregate runner time, and final workflow duration when expanding the matrix. | Missing focused inventory/cost check |

Run `actionlint` when editing workflows. Keep full-suite validation during rollout;
adding these cases does not require changing required checks or branch protection.

## 8. Performance cases and measurement

These are P1 and currently missing as repeatable timing comparisons. Selection
correctness is a prerequisite for accepting any measured saving.

| ID | Workload | Required evidence |
| --- | --- | --- |
| PERF-01 | Many independent modules with a localized source change | Less total time when most expensive tests are excluded; record selected modules and actual executed tests. |
| PERF-02 | Shared core module with many dependents; several modules changed together | Measure the small or absent saving as selection approaches the full suite. |
| PERF-03 | A large single module with many tests | Record that source changes still run its entire suite; quantify selector/setup overhead. |
| PERF-04 | Documentation-only change, no change, and build/configuration change | Measure `NONE` including native `clean` startup, and measure `ALL` including selection overhead. |
| PERF-05 | Excluded Kafka/Redis module versus an affected service consumer | Count service/container starts and include startup, image handling, shutdown, and tests in total time. |
| PERF-06 | Compilation-dominated build versus test-dominated build | Separate compile/package/analysis time from test time. Show the work remaining in excluded modules. |
| PERF-07 | Cold dependencies/images, warm dependency caches, and enabled native task-output caching | Treat these as separate conditions. Compare equivalent native tasks and equivalent cache states; account for forced `clean`. |
| PERF-08 | Fresh runner versus cached Sieve executable; consumer job versus this repo's validation workflow | Include checkout/history fetch, Rust/tool installation, Sieve build, native build, reports, and any subsequent full-suite stage. Report time to selected feedback separately from final CI completion. |
| PERF-09 | Representative real repository changes | Replay source, test, docs, dependency, and shared-input changes. Measure frequency of `ALL`, weighted time savings, and any missed failures. Do not extrapolate from the toy suite alone. |

Use the existing native full build as the baseline. Also time `sieve run --full`
to isolate runner overhead from the benefit of selection. Compare identical build
goals, profiles, test inventory, JDK/tool versions, runner resources, and commit.
If Sieve adds lifecycle work to the original command, report that cost explicitly.

For a warm-cache comparison, warm each condition and measure at least five paired
runs, alternating order. Preserve raw elapsed times and report median and range.
Measure cold runs separately. Do not run competing builds concurrently on the
measurement machine. Tiny changes within timing noise are inconclusive.

Store commit/base SHAs, commands, environment/cache state, selection mode/reason,
selected modules, executed tests, observed failures, container starts, and elapsed
times with the result. Report both wall-clock duration and aggregate runner time
for parallel CI. No speed percentage should be inferred from test counts alone.

## Suggested implementation order

- [ ] P0: DEP-02 through DEP-07: deeper graphs, runtime/resource/test dependencies,
  graph changes, and the stale-graph safety gap.
- [ ] P0: GIT-03 through GIT-06 and GIT-09: isolated deletion, committed moves,
  PR merge semantics, missing history, and shared inputs outside the workspace.
- [ ] P0: RUN-01 through RUN-05: real native failures, clean report isolation,
  and deliberate attempts to fool the verifier.
- [ ] P0: BUILD-02 through BUILD-08 and CI-01 through CI-06: installation,
  unsupported-layout rejection, native filtering, and event/report behavior.
- [ ] P0/P1: INT-02 through INT-05, then PERF-01 through PERF-09: selected container
  execution, proven avoided startup, and measurements including CI overhead.
- [ ] P1/P2: Remaining filename, concurrency, reporting, and consumer-specific
  framework cases as the core coverage stabilizes.

Class-level analysis, runtime tracing, previous class-dependency graphs, and
selection-cache invalidation remain future capabilities. Add their dedicated
coverage when those capabilities are implemented; do not count them as current
support or require them to validate this module selector.
