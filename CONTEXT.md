# Sieve

Conservative test selection for Java and Kotlin/JVM builds: run only the tests a change can affect, and fall back to the full suite when unsure.

## Language

### Measurement

**Feedback time**:
Wall-clock time of one `sieve run`, from start to verdict, covering selection, build, and test execution.
_Avoid_: testing time, test time, build time

### Tests

**Context test**:
A test that starts an application context, such as `@SpringBootTest`, `@MicronautTest`, or `@QuarkusTest`.
_Avoid_: container test

**Slice test**:
A context test that loads only some kinds of components, such as `@WebMvcTest` or `@DataMongoTest`.
_Avoid_: partial context test

**Testcontainers test**:
A test that starts external services in Docker.
_Avoid_: container test, Docker test

### Selection evidence

**Test record**:
What one test class has done across its passing runs since the class itself last changed: every method it ran, with bytecode hashes, and every classpath resource it read, plus whether its latest run passed.
_Avoid_: recording, coverage, trace, snapshot

**Base**:
The commit a change is compared against. Skipping a test on static grounds assumes the base's tests pass.

**Related test file**:
A JavaScript or TypeScript test file that is a changed file or imports one, directly or transitively, as the test runner (Jest or Vitest) resolves its imports.
_Avoid_: affected test, impacted test

**Source root**:
A directory of a JavaScript or TypeScript package, declared in `impact.json`, whose script files Sieve maps to related test files; a change anywhere else selects every test.
_Avoid_: roots (Jest's `roots` option is where Jest looks for files)
_Avoid_: baseline, reference

**Body change**:
An edit confined to method bodies that leaves signatures, fields, annotations, and the type hierarchy intact.
_Avoid_: internal change, implementation change

**Structural change**:
An edit to a class's shape: its signatures, fields, annotations, or supertypes, or the addition or removal of a class.
_Avoid_: API change

**Use** (of a class):
Running code of the class other than its constructors and static initializer, or reading or writing its fields. A Spring context constructs every component; only tests that use a component depend on its state.
_Avoid_: touch, load

**Plain constructor**:
A constructor or static initializer whose bytecode only stores values into the class's own fields, so its effects reach a test only through code of the class.

**Startup check**:
One full context test that runs when a narrowed change could stop the context from starting and no selected test would show it.
_Avoid_: canary, smoke test

### Validation

**Missed failure**:
A test that fails in the full run but was not executed by the selected run.

**Edit catalog**:
A fixed sequence of typical edits applied to a project's HEAD, each timed as a warm local run and weighted by how often that kind of edit occurs in the project's history.
_Avoid_: benchmark (the fixture benchmark is a different tool)

**Commit walk**:
A replay that applies a project's recent commits in order to one working tree, carrying test records and build output forward.
_Avoid_: cold replay (which rebuilds each commit from a clean tree)

**Planted bug**:
A fault injected into a changed method body while replaying history, to check that the selected run fails whenever the full run fails.
_Avoid_: mutation (fixture scenarios use that word), seeded fault
