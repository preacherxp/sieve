# A single-module selection with no useful saving

Changing `Price` selects this Maven project's only module. Module selection runs
both `PriceTest` and `SlowUnrelatedTest`, exactly like a full build, so skipping
modules provides no useful saving. The slow test sleeps for six seconds as a
controlled stand-in for unrelated application/container setup. It does not measure
a real service.

After `sieve init`, explicitly set `"class_level": true` for this measured workload.
The existing bytecode selector finds that only `PriceTest` reaches `Price`, so the
unrelated test is skipped. This reuses the existing selector and its conservative fallbacks;
it does not add a cache or require the local-mode agent.

From the repository root:

```bash
cargo build --release --locked
python3 scripts/benchmark-selective-performance.py --maven /path/to/mvn
```

The script creates temporary copies, runs `sieve init` once per copy, commits each
configuration before changing `Price`, warms three conditions, then alternates
five samples per condition. Each run starts from `clean`; wall time includes
Sieve, compilation and JVM startup. The class condition explicitly enables
class selection; the module condition retains the setup default. Native full and module
selection must execute both test cases; class selection must execute only
`PriceTest`. The oracle reads Maven XML rather than trusting selection JSON.
The script also verifies missing-history full fallback and checks that the same
deliberate price bug fails both native full and selected execution.

Raw logs, selections and `summary.json` go under the ignored
`validation-results/selective-performance/` directory. Build failure or an
incorrect test inventory prevents a timing summary. Timing is reported, not
asserted: machine noise must not make a correctness check flaky.

Use `--delay-ms 0` to expose the limit: with trivial tests, the second build-tool
startup for bytecode selection can cost more than skipping a test saves. Set
`"class_level": false` for such a project; `sieve refresh` preserves that choice.
These are synthetic warm-dependency measurements, not evidence of a general CI
speedup. No workflow or full-suite rollout check is changed.

The [measured results](../../docs/performance.md#single-module-setup-reproduce-no-gain-then-select-test-classes)
show five-run medians of 7.41 seconds full, 7.46 seconds module-selected, and
2.31 seconds class-selected with the six-second delay (69% saving). With no
delay, class selection is slower: 2.21 seconds versus 1.38 seconds full.
