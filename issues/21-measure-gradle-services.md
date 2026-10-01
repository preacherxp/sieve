# 21 Measure on the Gradle services

Type: HITL

## Parent PRD

[15](15-prd-local-mode-for-gradle-and-java-17.md)

## What to build

Final evidence on the two single-module Gradle services measured on 2026-09-30. Results, catalogs,
and service names stay outside this repository.

- Write an edit catalog per service from its history (`sieve classify`) and run it with `--plant`.
- Walk 20 recent commits per service with `--plant`.
- Compare weighted feedback time with the native `Test` tasks on a warm daemon, and with the
  2026-09-30 static numbers.
- Note the service-side fixes that would pay off next, such as a flaky test with colliding random
  data, or a database container started during configuration.

## Acceptance criteria

- [ ] No missed failure in the catalogs and walks; known flaky tests are listed separately.
- [ ] Weighted feedback time per service, next to the native run, with phase breakdowns.
- [ ] A decision on the Gradle defaults (daemon, default tasks), recorded in this issue.

## Blocked by

- 16
- 19
- 20

## User stories addressed

- User stories 1–6
