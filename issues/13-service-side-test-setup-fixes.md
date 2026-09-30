# 13 Service-side test setup fixes

Closed 2026-09-30: per-service audits are done and kept privately; applying them in the service repositories moved to [TODO.md](TODO.md).

Type: HITL. The work happens in the services' own repositories, not here.

## Parent PRD

[00](00-prd-local-first-test-records.md)

## What to build

Remove the costs that Sieve cannot remove, and make the services meet ADR 0001's independence
assumption. For each benchmark service:

- Start each container once per JVM instead of once per test class.
- Remove `@DirtiesContext` from shared test base classes.
- Isolate each test's data: unique topic and consumer-group ids, collection cleanup.
- Mark shared containers `withReuse(true)` so that local runs keep them between runs; issue 10
  switches reuse on.
- Where the catalog shows many contexts, merge `@MockitoBean` sets into a few shared test
  configurations.

## Acceptance criteria

- [ ] Each service's full suite passes in both test-class orders, before and after the change.
- [ ] Private catalog results show full and selected feedback time before and after.
- [ ] No service-specific detail is committed to this repository.

## Blocked by

None - can start immediately.

## User stories addressed

- Supports user stories 2 and 9 through the independence assumption
