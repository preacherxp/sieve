# 06 Context startup and Spring wiring

Closed 2026-09-30: implemented; the test-class order check moved to [TODO.md](TODO.md).

Type: AFK

Status (2026-09-30): implemented with a `ContextCustomizerFactory`, a Spring
`TestExecutionListener`, and a `SpringApplicationRunListener` (Boot reads its configuration
before customizers run). Tested: listener body, bean constructor, added component. Not yet
tested: both test-class orders; the phase breakdown waits for issue 01. Lazily created beans
count only for the test class that first used them.

## Parent PRD

[00](00-prd-local-first-test-records.md)

## What to build

Make context tests eligible for record-based skipping, because that is where the expensive tests are.

- A Spring TestContext hook inside the agent jar, registered automatically when Spring's test support
  is on the classpath, marks each context's startup. Methods executed while a context starts count for
  every test class that uses that context, so a change to a bean constructor or `@Bean` method reruns
  them all.
- Spring wiring edits select every context test whose kind loads them, reusing today's slice-aware
  rule. Wiring covers stereotype annotations, `@Bean`, `@Configuration`, `@Conditional…`,
  auto-configuration imports, and added or removed components.
- Context tests become subject to the same skip conditions as other tests.
- `projects/records` gains a Spring Boot part: two full-context test classes sharing one context, a
  slice test, and a listener that the framework invokes asynchronously on an event, with no bytecode
  edge from the tests to it.
- Issue 01's phase breakdown gains context and container startup time.

## Acceptance criteria

- [ ] A body edit in the framework-dispatched listener reruns only the test class that triggered it.
- [ ] A bean constructor edit reruns every test class sharing that context, including those that ran
      after the context was cached.
- [ ] Adding a component reruns every full-context test and only the slices that load its kind.
- [ ] All new scenarios pass with zero missed failures, in both test-class orders.

## Blocked by

- 04

## User stories addressed

- User stories 2 (context tests), 8, 9, 21
