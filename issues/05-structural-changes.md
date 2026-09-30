# 05 Structural changes invalidate records per class

Closed 2026-09-30: implemented; the remaining oracle scenarios moved to [TODO.md](TODO.md).

Type: AFK

Status (2026-09-30): implemented (shape and hierarchy check against a per-record class
snapshot). Tested: an added override called through the supertype. Not yet tested: changed
method annotation, removed method, supertype change.

## Parent PRD

[00](00-prd-local-first-test-records.md)

## What to build

Replace issue 04's "any structural change runs everything" with a per-class rule. A structural change
to a class (its signatures, fields, annotations, supertypes, access flags, added or removed methods, or
the removal of the class) reruns every test whose record touches that class or any of its project
supertypes or subtypes. The hierarchy matters because a new override can change dispatch without
changing any method a test executed.

Added classes need no rule of their own: tests reach them only through changed code, a string
constant (existing static rule), or Spring wiring (issue 06).

## Acceptance criteria

- [ ] New oracle scenarios pass with zero missed failures: an added override of a method that tests
      call through the supertype, a changed method annotation, a removed method, and a supertype
      change.
- [ ] A structural change to a class that no record touches selects nothing beyond the static rules.
- [ ] `--output` names the structural change as the reason.

## Blocked by

- 04

## User stories addressed

- User stories 7, 21
