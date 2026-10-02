---
status: proposed
---

# Constructing a component is not using it

Local mode was meant to run only the tests that a change can affect, but on the clinic benchmark (ADR 0003 measurements, 2026-10-01) 14 of 20 commits reran almost every integration test and saved 2–9%. The records were right; the rules reading them were too coarse for Spring. Every context starts by constructing every component, so every context test's record contained every bean constructor, and three rules turned that into "rerun everything": a changed constructor counted as a changed method of every test; a structural change (a new field, annotation, or method) of any constructed class counted for every test that constructed it; and any changed or added component reran every context test. Edits to `application.yml` and new Flyway migrations did the same through the file and directory-listing records.

A test now depends on a class's state and shape only when it ran code of that class other than its constructors and static initializer, or used its fields. Constructors and static initializers whose bytecode only stores values into the class's own fields ("plain") affect the class's users and nothing else. Component wiring is compared per part: the declaration (supertypes, class annotations), the constructors, and each annotated member. Changes whose annotations act only on calls to the component, such as `@Transactional`, `@Audited`, `@Query`, or a new constructor dependency, rerun the tests that use the component; request mappings also rerun the users of handlers whose paths may overlap; listeners, `@Bean` methods, lifecycle callbacks, unknown annotations, and changed declarations keep the previous rule. A changed configuration file reruns the users of project classes that name a changed key (placeholder, key constant, or bound prefix); a key that no project class names is read by the framework, and every test reading the file runs. A migration directory that only gained appended versioned migrations of inert statements (non-unique indexes, sequences, comments, tables without foreign keys) reruns nothing.

Each of these narrowed changes can still stop the context from starting, which every context test would show. When none of the selected tests starts a full context, and no full context test has passed since such a change, one runs anyway: the startup check.

## Considered Options

- Keep context startup in every test's record: safe, but every component edit stays a full integration run, as measured.
- Record startup work separately and ignore it: cheaper still, but a constructor or `@Bean` method that configures shared state (an `ObjectMapper`, a security filter chain) would go unnoticed.
- Track configuration keys and SQL tables at runtime in the agent: more precise, but needs instrumentation of Spring's property sources and of JDBC drivers, and the test-suite base class that truncates every table makes table evidence useless anyway.

## Consequences

- Safety rests on one more assumption: a plain constructor's effects stay in its object. A constructor that mutates an injected collection or registry passes the bytecode check only when it calls nothing outside the JDK value types it allows, so such registration code keeps the old rule.
- A context that no longer starts fails only the selected tests and the startup check; the dropped context tests would fail the same way and are not reported as failures.
- Reflection that reads a class's members without running its code, such as Hibernate validating an entity at startup, is still only seen by the tests that use the class and by the startup check, as before.
- Kotlin's `SourceDebugExtension` annotation (a line map) is ignored in shapes, so line shifts in Kotlin files no longer look like structural changes.
- Snapshots and digests changed format: the first run after upgrading reruns every test.
