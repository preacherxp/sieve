# 02 Baseline the benchmark services

Type: HITL

## Parent PRD

[00](00-prd-local-first-test-records.md)

## What to build

Baseline numbers on the four internal benchmark services with today's Sieve, before any engine work,
to confirm which later slice pays off first.

First make sure Docker works and each service's suite passes locally; one service's latest local
reports show every test erroring. Then write a private catalog per service, derive weights from its
history, and run it against today's class-level `sieve run` and a native `mvn verify`.

## Acceptance criteria

- [ ] Each of the four services runs its full suite green locally, or the blocker is recorded.
- [ ] Private catalogs and results exist outside this repository.
- [ ] A short private summary names the dominant phase per service and states whether the order of
      issues 05–10 should change.
- [ ] Nothing internal (service names, packages, results) is committed to this repository.

## Blocked by

- 01

## User stories addressed

- User story 18
