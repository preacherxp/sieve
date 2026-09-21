# Java test-impact validation samples

Two independent, equivalent Java 17 projects: **Maven + Surefire/Failsafe** and
**Gradle + separate unit/integration test tasks**. Use them as a correctness and
precision benchmark while building a test selection engine.

This bundle contains fixtures and a validation harness. It does **not** implement
the selection engine or claim that the proposed `impact` plugins already exist.

## Contents

- `projects/maven/`: three-module Maven reactor.
- `projects/gradle/`: equivalent three-project Gradle build.
- `scenarios.json`: 17 reproducible mutations, required selections, and expected failures.
- `validate.py`: prepare workspaces, apply mutations, check selections, parse reports, verify full runs.
- `test_harness.py`: tests for mutation application, parity, selection checks and report parsing.
- `.github/workflows/fixtures.yml`: Maven and Gradle validation jobs in parallel.
- `VALIDATION.md`: results and environment used when creating this bundle.

## Prerequisites

JDK **17** (a full JDK, not just a JRE), Python **3.10+**, Maven **3.9.9** and
Gradle **8.12.1**. Use installations on PATH or pass `--maven` / `--gradle` executable
paths. Git is optional unless using `prepare --git`. First builds need access to
Maven Central. Dependencies are pinned to JUnit 5.11.4 and Spring Framework 6.1.16;
these are reproducible fixture versions, not recommendations for production.

No database, Docker, credentials, remote service, or IDE is required. Standard build
tool installations are used; wrapper binaries are not included.

## Run the baselines

From the bundle root:

```bash
python3 validate.py verify --tool both
```

Or use the build tools directly:

```bash
cd projects/maven
mvn clean verify
```

```bash
cd projects/gradle
gradle clean check
```

Use Maven **verify**, not just test, to include Failsafe integration tests. Gradle
**check** includes the separate integrationTest tasks. Each baseline has **12 test
classes / 13 test invocations**; the parameterized test has two invocations.

## Validate all fixture mutations

```bash
python3 -m unittest -v test_harness
python3 validate.py list
python3 validate.py verify --tool both --scenario all
```

The harness creates isolated temporary projects and runs the **full suite** for each
mutation. It checks that every expected test class actually ran and that the failing
classes exactly match the manifest. Build/compilation errors fail validation. Reports
and build logs are written under `validation-results/`.

Mutations deliberately break behavior. A PASS in this harness means the expected
failures were observed; it does not mean the mutated application is correct. Test
failure ignoring is enabled only by the harness to let all modules finish. Ordinary
`mvn verify` / `gradle check` still fail normally on test failures.

## Validate your selector

The important sequence is **baseline run → mutation in the same workspace → selection**.
This preserves the baseline metadata your plugin creates.

```bash
python3 validate.py prepare --tool maven --dest /tmp/impact-maven-tax --git
```

1. Add/configure your Maven adapter in that workspace, then commit those setup changes.
2. Run your tool's full baseline/recording mode from that workspace.
3. Apply the mutation from the bundle root:

```bash
python3 validate.py apply tax-transitive --workspace /tmp/impact-maven-tax
```

4. Run your tool's selection mode in the workspace and export its selected class IDs
   to `actual-selection.json` using the format below.
5. Check that selection:

```bash
python3 validate.py check-selection tax-transitive --actual actual-selection.json
python3 validate.py check-selection tax-transitive --actual actual-selection.json --exact
```

Repeat with `--tool gradle` and a separate destination to compare adapters. The
`prepare` command refuses to overwrite an existing directory; `apply` refuses to
stack scenarios. Prepared mutations remain uncommitted, deliberately exercising
working-tree change detection. Commit after mutation to test committed-change handling.

### Selection interchange format

```json
{
  "mode": "SUBSET",
  "tests": [
    "pricing:unit:example.TaxRulesTest",
    "pricing:unit:example.PriceCalculatorTest",
    "checkout:unit:example.CheckoutTest",
    "checkout:unit:example.ParameterizedCheckoutTest"
  ]
}
```

IDs are `module:suite:fully.qualified.ClassName`. Suites are `unit` or `integration`.
`{"mode":"ALL","tests":[]}` selects the entire currently eligible inventory;
`{"mode":"NONE","tests":[]}` selects none. SUBSET requires unique known IDs.
Selection is class-level, so parameterized invocations share one class ID.

The default check is **safety-oriented**: all `required` IDs must be present, while
additional tests are reported as `extra`. A conservative full-suite fallback can pass.
`--exact` additionally rejects extras and measures precision. The minimum required
sets are behavioral requirements, not promises about the output of every static
analysis strategy. For dependency upgrades the required set intentionally encodes a
conservative suite-level invalidation policy.

Compare actual execution too:

```bash
python3 validate.py reports --workspace /tmp/impact-maven-tax --tool maven
```

The report command prints executed/failed/skipped IDs and invocation count. Clear
previous XML reports before the selection run without deleting your selector baseline;
otherwise stale reports can falsely suggest a skipped test ran. The `verify` command
avoids this problem by using fresh workspaces and clean builds. A selection JSON alone
does not prove that your adapter actually applied its filters.

## Fixture modules

| Module | Production behavior | Tests |
|---|---|---|
| pricing | Tax rules, pricing, independent currency label, unused discount | 3 unit classes |
| checkout | Depends on pricing; interface-based payment gateway; receipt | 4 unit classes, including inherited fixture and parameterization |
| runtime | Spring bean configuration, reflection, properties, executor, ServiceLoader | 5 integration classes |

Maven and Gradle contain byte-for-byte identical Java/resources. Each can be copied
out and built independently; neither references source directories in the other.

## Scenarios

| ID | What it validates | Minimum selected classes |
|---|---|---|
| tax-transitive | Leaf change propagates across module boundaries | TaxRulesTest, PriceCalculatorTest, CheckoutTest, ParameterizedCheckoutTest |
| calculator | Shared calculation propagates downstream | PriceCalculatorTest, CheckoutTest, ParameterizedCheckoutTest |
| unrelated | Independent feature isolation | CurrencyLabelTest |
| unused | Unused production class | None |
| gateway-implementation | Interface implementation change | GatewayTest, CheckoutTest, ParameterizedCheckoutTest |
| inherited-fixture | Test superclass dependencies | CheckoutTest, ParameterizedCheckoutTest |
| changed-test | Modified test must run | TaxRulesTest |
| new-test | Test absent from baseline must run | NewTaxTest |
| reflection | Class name lookup with no normal type reference | ReflectionIT |
| async | Method executes on a worker thread | AsyncIT |
| spring-bean | Bean implementation also used through ServiceLoader | SpringWiringIT, ServiceLoaderIT |
| spring-wiring | Configuration selects a different implementation | SpringWiringIT |
| resource | Properties file affects behavior | ResourceIT |
| service-provider | Provider registration is a resource dependency | ServiceLoaderIT |
| delete-class | Removed class with compilable consumer replacement | ReceiptTest |
| docs-only | Documentation does not invalidate Java tests | None |
| dependency-change | Changed resolved Spring dependency | All runtime integration classes |

The delete-class case also changes its consumer; it catches broken deletion handling
but cannot by itself prove that the engine traverses the previous graph. Reflection
and resource cases may correctly trigger a broader integration-suite fallback in v0.1.

## Next validation layers

These small projects establish a first benchmark, not a complete soundness proof.
Add the following when those capabilities enter the tool's supported contract:

- Missing/corrupt/version-incompatible baseline: explicit full-suite fallback.
- A → B → A checkout and interrupted runs: no stale success state.
- Parallel tests and pre-existing executors: attribution without ThreadLocal assumptions.
- Shared Spring contexts, dynamic tests, JUnit extensions and custom engines.
- Annotation processors, generated classes, constant inlining and incremental compilation.
- Renames and deletion that change classpath discovery without editing test source.
- Duplicate class names across suites and classloaders; partial reactor builds.
- Cached subset versus full-suite execution; fail-on-no-tests versus intentional NONE.

Never replace `required` with your engine's output just to make the benchmark pass.
When changing a fixture, update the manifest only after independently checking its
behavior with a full-suite run.

## Can an import graph drive selection?

An import graph can be a cheap preliminary approximation, but it is not a complete
Java dependency graph. Imports resolve names; they do not enumerate every dependency.
This fixture deliberately puts many related classes in the same package, so an
import-only selector fails the `tax-transitive` case even though the calls are ordinary
Java calls.

| Mechanism | What it can tell you | Limitation |
|---|---|---|
| Parsed import statements | Explicit imported names and packages | Same-package and fully qualified references need no import; unused imports add noise |
| Source AST with symbol resolution | Actual statically resolved references | Needs correct classpath/source roots; framework and reflection edges remain |
| Compiled bytecode graph | References after compilation, including generated code | Inlined constants and dynamic discovery still require additional handling |
| Runtime dependency recording | Dependencies exercised by observed runs | Previously unexecuted paths and attribution across threads/processes remain |

Recommended validation order: use `tax-transitive` to prove same-package dependency
handling, `gateway-implementation` for implementation dispatch,
`reflection` / `spring-wiring` / `service-provider` for dynamic behavior, and
`resource` for non-Java inputs. Use an import graph for hints; use semantic references
and conservative fallback rules before deciding that a test can be skipped.
# sieve
