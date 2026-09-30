---
status: accepted
---

# Record runtime evidence to narrow context tests

Static analysis cannot rule out any `@SpringBootTest` for an edit to a Spring component: framework dispatch (Kafka listeners, HTTP handlers) leaves no bytecode edge from the test to the code it runs, and on the team's Spring Boot services those tests carry most of the test time. Every run therefore keeps a test record per test class (the methods it executed with their bytecode hashes, the classpath resources it read, and its outcome), and a test is skipped when its last run passed and none of its recorded methods or resources changed, or when static analysis shows it reaches no change since a green base. Structural changes and tests without a passing record keep the static rules. Records are produced on developer machines first; CI comes later. This reverses the earlier "no runtime recording agent" stance and ships a JVM-side component, so Sieve is no longer Rust-only.

## Considered Options

- Static analysis only: keeps the stronger guarantee, but every edit to a Spring component still runs every context test.
- One full recording refreshed by hand: simpler, but it goes stale between refreshes and costs extra full runs.
- Predictive selection from failure history: larger savings, but it accepts missed failures, which contradicts Sieve's conservative purpose.

## Consequences

- Safety rests on stated assumptions: every path a test can take has shown up in one of its passing runs since the test class last changed, and tests do not depend on side effects left by other tests. Records therefore accumulate across passing runs instead of keeping only the latest one.
- Class-level recording would not help, because starting a Spring context loads every bean class. Records are per method, and work done while a context starts counts for every test that shares that context.
