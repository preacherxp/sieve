# Kotlin shipping (Maven)

A Kotlin-only Maven example with two modules built by `kotlin-maven-plugin`: `rates`
holds an `object` of `const val` fees and an enum with a companion lookup; `orders`
prices a data-class `Parcel` with an extension property. Three test classes, five
invocations, no external services.

```bash
cd samples/kotlin-shipping
mvn clean verify
sieve run --workspace . --full
# In a standalone Git copy, commit the baseline, edit a source, then:
sieve run --workspace . --base HEAD
```

`impact.json` and the per-module `impact.skip` properties were generated with
`sieve init`; class-level selection is enabled:

| Change | Module-level | Class-level |
| --- | --- | --- |
| `rates/Rates.kt`, a `const val` | all 3 | `QuoteTest` |
| `rates/Zone.kt` | all 3 | `ZoneTest`, `QuoteTest` |
| `orders/Parcel.kt` | `orders` (2) | `ParcelTest`, `QuoteTest` |

`kotlinc` copies `const val` values into callers, so `Quote` keeps no reference to
`Rates`. Sieve finds it because `Quote.kt` names the constant's owner. From the
repository root:

```bash
IMPACT_TOOL=maven cargo test --locked --test native native_sample_mutations -- --ignored --nocapture
```

The independent oracle checks native full builds and both selection modes for a
changed constant, an enum lookup bug, an extension-property bug, a test edit, a
build-file edit, and a docs-only edit, with exact classes, invocation counts, and
failures. `IMPACT_MAVEN` overrides `mvn`.
