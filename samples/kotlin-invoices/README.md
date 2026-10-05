# Kotlin invoices (Gradle Kotlin DSL)

A Kotlin-only example with two modules, configured with `build.gradle.kts`: `money` has
a value class, an extension function, and an `inline` sum; `invoice` uses them with a
sealed `Discount` hierarchy and data classes. Four test classes, six invocations, no
external services.

```bash
cd samples/kotlin-invoices
gradle clean check
sieve run --workspace . --full
# In a standalone Git copy, commit the baseline, edit a source, then:
sieve run --workspace . --base HEAD
```

`impact.json` was generated with `sieve init` and enables class-level selection, which
handles what Kotlin compiles away:

| Change | Module-level | Class-level |
| --- | --- | --- |
| `money/Sums.kt`, the `inline` body | all 4 | `SumsTest`, `InvoiceTest` |
| `money/Format.kt` | all 4 | `FormatTest` |
| `invoice/Discount.kt` | `invoice` (2) | `DiscountTest`, `InvoiceTest` |

`Invoice` calls `sumOfCents`, but its bytecode holds a copy of the inline body, not a
call. Sieve finds the copy through the Kotlin source map (`SourceDebugExtension`) that
records where inlined code came from. From the repository root:

```bash
IMPACT_TOOL=gradle cargo test --locked --test native native_sample_mutations -- --ignored --nocapture
```

The independent oracle checks native full builds and both selection modes for an
inline-function bug, a formatting bug, a sealed-branch bug, a test edit, a build-file
edit, and a docs-only edit, with exact classes, invocation counts, and failures.
