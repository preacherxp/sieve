# Sieve JVM improvement research

Research date: 2026-10-12. Source snapshot: Sieve 0.3.0 at commit a85be4f007589bee4f8366ea52a2b2c3ce4f5f63 in /Users/prchr/Documents/ai/sieve.

**Recommendation: add live shadow validation and independent failure-bearing evidence first.** Then address a real adopter's build or runtime restrictions. Sieve already has substantial selection machinery; the largest remaining opportunities concern confidence, integration, and workload eligibility.

The ranking below is a product judgment based on source inspection and primary documentation, not measured customer demand. Proposed names such as “shadow mode” and “must-run policy” are new concepts, not existing CLI options.

## What Sieve already does

Module selection, bytecode class selection inside the build, Kotlin inline maps, Spring context/slice handling, runtime method/file/jar records, dependency-version narrowing, per-test explanations, incremental local runs, history replay, planted bugs, and readable reports already exist. Portable CI records also exist, but are explicitly experimental and unsupported for consumer rollout. These should not be presented as missing features. [Selection reference](reference.md#class-level-selection), [runtime records](reference.md#local-mode-test-records), [CI records](reference.md#records-in-ci).

## Ranked opportunities

| Priority | Opportunity | Current gap | Smallest useful delivery | Scope |
| --- | --- | --- | --- | --- |
| 1 | Shadow validation and independent regressions | Replay exists; live full-suite observation and diverse external failure evidence are missing | Freeze a proposed subset, run the full suite once, report missed failures; publish real regression cases | Medium |
| 2 | Persistent must-run rules | Explicit test requests exist; an additive persistent selection override does not | Class-name patterns that are added to each normal selection | Small to medium |
| 3 | Preserve native tasks and reports | Static execution defaults to clean verify/check; consumer coverage/custom-task integration is incomplete | Support one adopter's exact task graph, reports and NONE behavior | Medium |
| 4 | Gradle convention-plugin layouts | Setup rejects included builds and relocated project directories | Support bounded build-logic includes, with conservative invalidation | Medium |
| 5 | Runtime records beyond sequential single-module projects | Multi-module records are rejected; concurrent classes prevent trusted recording | Pick one restriction blocking a real pilot and remove it with measured evidence | Large |
| 6 | Mode diagnostics and compatibility evidence | Some fallback reasons are vague/stale; current version matrix concentrates on module selection | Precise active-mode reasons and focused class/records engine fixtures | Small to medium |
| 7 | Execute the remaining tests later | Selected-plus-full validation duplicates tests | Verified execution receipt and complement execution at identical inputs | Medium to large |

Scope estimates are relative engineering judgments, not delivery commitments.

## 1 Shadow validation and independent regressions

The published Commons Text history contains no failing full-suite runs. The larger Spring service is a purpose-built benchmark authored alongside Sieve. Both are useful, but do not establish failure retention across unrelated services. [Existing evidence](performance.md).

Launchable documents an observation mode that executes all tests while retaining the proposed subset for comparison. This is a useful model for Sieve's next feature. [Official observation documentation](https://help.launchableinc.com/features/predictive-test-selection/observing-subset-behavior/).

**Proposed delivery:** after compilation, freeze the selection before executing tests or refreshing records. Run the native full suite once and compare failures against that frozen decision. Report missed failing classes, whether a failing build would still have been detected, full-selection reasons, and runs without failures. Start with supported static CI modes; experimental runtime CI should retain its current status.

Observation establishes which full-run failures fall outside a proposal. It cannot expose failures caused only by subset ordering or shared state, and its duration estimates are not measured selected-run wall time. Keep paired selected/full runs for those questions.

Add independent regression cases using existing replay infrastructure. Warm records on a fixed revision, keep fault-revealing tests unchanged, then revert only the production fix. Otherwise a newly added or edited test can trivially force selection and weaken the experiment. Pin toolchains, publish exclusions, repeat suspicious failures, and retain infrastructure/flake outcomes as inconclusive.

RTSCheck provides a research precedent: generated program evolution, real buggy/fixed pairs, and histories exposed 27 reported bugs across Clover, Ekstazi and STARTS. Its artifacts are useful test-case sources, not a turnkey modern JVM benchmark. [Original ICSE paper](https://www.cs.cornell.edu/~legunsen/pubs/ZhuETAL19RTSCheck.pdf), [artifact](https://github.com/EngineeringSoftware/rtscheck).

**Acceptance:** a deliberately broken selector produces a visible miss; zero failing runs produce “no failure-retention evidence”; paired replay keeps full-only and selected-only failures distinguishable; reported feedback-time gains come from equivalent paired executions.

## 2 Persistent must-run rules and external inputs

Sieve already lets users force explicit tests and declare environment inputs through record_env. Its configuration has no persistent additive class-pattern policy. Runtime evidence excludes undeclared variables, external services, floating image tags, and other agents outside the test classpath. [Configuration](../src/main.rs#L25), [limits](reference.md#local-mode-test-records).

A useful policy would union matching test classes into ordinary selections, including otherwise empty selections. A remote API contract check or environment smoke test could then run every invocation without disabling selection for the entire project. Define interactions with native exclusions, explicit test requests, and the no-build shortcut. Develocity supplies a comparable MustRun annotation. [Official annotation project](https://github.com/gradle/develocity-testing-annotations).

**Start with class patterns**, using the existing configuration and explanation output. Automatic observation of environment-variable reads and explicit external-file/service-version fingerprints can follow if pilots need them. Gradle itself documents the need to declare environment variables as task inputs, so relying on the build tool does not remove this boundary. [Gradle input guidance](https://docs.gradle.org/current/userguide/common_caching_problems.html#environment_variable_tracking).

A bounded additional improvement is hashing the contents of other Java agent jars: JVM arguments already count, but replacement at the same path can leave their argument string unchanged. This is a documented evidence limitation, not a reproduced missed failure. [Argument hashing](../src/records.rs#L1615), [documented excluded inputs](reference.md#local-mode-test-records).

**Acceptance:** an unchanged must-run class executes on an otherwise NONE run; declared external-input changes invalidate before the shortcut; unknown external state is not advertised as tracked.

## 3 Preserve the native build contract

Static mode constructs clean verify or clean check, scoping partial Gradle runs to module check tasks. Local mode already supports incremental execution and task overrides. The coverage plan marks preservation of arbitrary consumer coverage, packaging and custom tasks as incomplete. [Runner](../src/main.rs#L541), [BUILD-10](test-coverage.md#4-setup-and-native-build-adapters).

This affects ordinary projects: Gradle's additional JVM test suites do not automatically join check. Their guide explicitly wires integrationTest into it. Sieve already handles custom Test tasks; the missing piece is preserving which tasks the user's CI actually requests. [Official JVM Test Suite guide](https://docs.gradle.org/current/userguide/jvm_test_suite_plugin.html).

**Proposed delivery:** make the native goals/tasks explicit for a concrete adopter and show the resulting command and relevant test/report inventory. Specify how NONE behaves when downstream jobs still require compiled artifacts or reports. Keep partial coverage distinguishable from full-suite coverage.

Benchmark against the adopter's real cached, incremental command as well as the existing clean comparison. A clean removes up-to-date output reuse; it does not by itself prove Gradle build-cache restoration cannot work.

**Acceptance:** test integrationTest, report aggregation, non-test checks, excluded modules and NONE against native task/output expectations. Measure end-to-end feedback time, not only skipped-class duration.

## 4 Support Gradle convention-plugin layouts

The init script rejects nonempty includedBuilds and project directories that differ from project-path-derived locations. This is a confirmed setup restriction. [Included-build rejection](../src/gradle.init.gradle#L123), [directory mapping](../src/gradle.init.gradle#L141).

Gradle documents build-logic included builds as a standard way to share convention plugins. Such a project can be a normal Java/Kotlin application even though it uses a composite build. [Official build-logic guide](https://docs.gradle.org/current/userguide/sharing_build_logic_between_subprojects.html#sec:using_a_composite_build_named_build_logic).

**Proposed delivery:** recognize a bounded class of plugin-only included builds; treat their sources, resources and dependency configuration as build inputs that conservatively widen selection. Preserve any plugin-build verification required by the original command. Establish explicit build identity and directory mappings before expanding to relocated projects.

This requires more than removing the rejection. Arbitrary application composites and cross-build dependency substitution remain a separate larger feature.

**Acceptance:** ordinary source edits select the expected tests; a convention-plugin edit selects conservatively; native plugin tests and task dependencies still execute; unsupported composite relationships are reported explicitly.

## 5 Broaden runtime record eligibility

Records explicitly require the single root module. Configured JUnit parallel execution bypasses selection; observed overlapping classes prevent retaining new trusted passing records. Existing records remain on disk. These limits prevent two classes of workloads from using Sieve's most precise mode. [Single-module guard](../src/records.rs#L2224), [parallel behavior](reference.md#local-mode-test-records).

Choose the restriction that blocks a pilot:

- Multi-module records need module/task/test identity, upstream bytecode evidence, correct output ownership, and isolated record namespaces.
- Concurrent-class records need correct attribution through executors, asynchronous callbacks and shared Spring contexts. The current probe uses a global current bucket; replacing it with ThreadLocal alone would not solve asynchronous ownership. [Probe state](../agent/probe/sieve/probe/Probe.java#L10).

Forked JVMs and concurrent classes inside one JVM are different cases; avoid claiming all parallel build execution is unsupported.

**Acceptance:** realistic cross-module or overlapping-class faults are retained, shared startup evidence remains correct, and the feature saves feedback time against the adopter's existing parallel execution. Keep conservative fallback for attribution that cannot be established.

## 6 Diagnose actual capability and verify modern engine combinations

Per-test explanations already exist. The smaller missing feature is a clear answer to “why did records not activate here?” One current summary conflates explicit requests, concurrency and JVM eligibility and still says Java 24+, although the documented minimum is Java 17. Agent-option paths containing whitespace are also explicitly rejected. [Stale summary](../src/records.rs#L2593), [path restriction](../src/records.rs#L2205).

A preflight could report the actual test JVM, engine/provider, eligible selection modes, selected task inventory and exact fallback reason. It should reuse existing checks, not become a second selector.

The compatibility workflow already includes modern JDK/Kotlin/Gradle combinations and Maven 4 RC. Its checks concentrate on installation and module selection. Expand a small matrix to exercise class selection and records with JUnit 6, nested/dynamic tests, Spock, Kotest, and TestNG provider variants. [Existing matrix](../.github/workflows/compatibility.yml#L48), [documented partial engine coverage](test-coverage.md#4-setup-and-native-build-adapters).

These are **unverified combinations, not proven incompatibilities**. Sieve uses JUnit Platform lifecycle/filter hooks; Spock/Kotest already use that platform. Current Surefire documentation says TestNG uses a Platform engine from 3.6.0, while native TestNG execution has a different integration boundary. [JUnit guide](https://docs.junit.org/6.1.3/overview.html), [Spock migration](https://spockframework.org/spock/docs/2.4/migration_guide.html), [Kotest setup](https://kotest.io/docs/framework/project-setup.html), [Surefire TestNG](https://maven.apache.org/surefire/maven-surefire-plugin/examples/testng.html).

**Acceptance:** baseline, irrelevant edit, relevant planted failure, discovery error and inactive-agent cases produce correct execution/reporting for each claimed mode. Add adapters only where those tests establish a need.

## 7 Run the remaining tests later

Sieve's performance documentation explicitly distinguishes earlier selected feedback from the extra cost of subsequent full validation. Develocity offers relevant and remaining-test modes to avoid repeated successful executions for the same change. [Sieve CI measurements](performance.md#actual-github-actions-cost), [Develocity guide](https://docs.develocity.ai/2026.3/guides/predictive-test-selection/#relevant_vs_remaining_tests).

**Proposed delivery:** emit a receipt of actual completed successful executions, bound to revision, input fingerprints, toolchain, invocation and current test inventory. A later stage runs the complement. Missing, stale, incomplete or mismatched receipts trigger a full run; planned-but-unexecuted tests cannot count as completed.

This optimizes execution; it does not validate the selector. Splitting execution may change test order and shared state. Keep independent full-suite checks while establishing equivalence.

**Acceptance:** the two stages collectively cover the current inventory; aborts, changed inputs, newly discovered tests and missing artifacts cannot produce a false complete result. Measure duplicated setup and total runner time as well as feedback time.

## Suggested next iteration

1. Implement shadow observation for supported static CI selection and run it on independent projects.
2. Publish a compact regression corpus with unchanged fault-revealing tests and actual failing full runs.
3. Ship persistent must-run patterns and precise inactive-mode diagnostics.
4. Use pilot results to choose native build integration, build-logic support, multi-module records, or concurrency.
5. Consider remaining-test execution after validation establishes the behavior.

Further Spring configuration/SQL precision, ML prediction, method-level test execution, Android/KMP support and incremental coverage reconstruction can wait for a measured need. No new implementation is necessary merely to enlarge the feature list.

## Research limits

Source and documentation inspection only. No Sieve or competitor benchmark, native compatibility build, or runtime bug reproduction was performed. Existing published measurements are cited with their original limitations. Source-code gaps are distinguished from compatibility questions and proposed product directions. Application code was not changed.
