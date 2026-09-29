# Bookstore: class-level selection in one module

A single Maven module with no internal module graph, so module-level selection can only
run everything. With `"class_level": true` in `impact.json`, `sieve run` compiles the
workspace, reads the bytecode, and runs only the test classes that reach a changed class.

Each test class waits 1.5 seconds in `@BeforeAll` (`TestEnvironment.boot`), standing in
for the per-class setup cost of real suites, such as starting a database container or an
application context. `-Dbookstore.setupMillis=0` removes the wait.

```text
catalog   Book, Catalog
pricing   PriceRule, MemberDiscount, BulkDiscount, TaxRules, PriceCalculator
orders    Inventory, Order, OrderService   (OrderServiceIT runs in Failsafe)
notify    EmailFormatter, Notifier
report    SalesReport
```

## Demo

`demo.sh` copies the sample into a temporary Git repository, runs the full suite, then
applies typical edits and runs `sieve run --base HEAD`:

```sh
cargo build --release
SIEVE=$PWD/target/release/sieve samples/bookstore/demo.sh
```

Recorded on 2026-09-24 with Maven 3.9.16 and Java 17 (times include both Maven runs):

| Change | Mode | Test classes | Time |
| --- | --- | ---: | ---: |
| none (`--full`) | ALL | 10 | 17 s |
| `notify/EmailFormatter` | SUBSET | 2 | 5 s |
| `pricing/BulkDiscount` | SUBSET | 3 | 7 s |
| `report/SalesReport` | SUBSET | 1 | 4 s |
| `catalog/Book` | SUBSET | 10 | 18 s |
| `README.md` | NONE | 0 | 1 s |

`BulkDiscount` selects `BulkDiscountTest`, `PriceCalculatorTest`, and `OrderServiceIT`,
which reach it through `PriceCalculator`. `PriceCalculator` only calls the `PriceRule`
interface, so a change to one implementation reaches callers of the interface, not
`MemberDiscount`, a sibling implementation. `Book` is used by every test, so its change
costs one extra compile over the full suite.
