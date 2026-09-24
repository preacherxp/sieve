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
