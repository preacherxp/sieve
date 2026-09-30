# 07 Record classpath resource reads

Closed 2026-09-30: implemented with dynamic probes instead of a static path-reader rule.

Type: AFK

Status (2026-09-30): implemented dynamically. The agent probes `FileInputStream`,
`RandomAccessFile`, and the default file-system provider, so classpath reads and reads by path
(`Files.readString`) are both recorded, and no static path-reader rule was needed. Since the review of
2026-09-30 the agent also probes existence checks and directory listings (`File.exists`,
provider `checkAccess`/`readAttributes`/`newDirectoryStream`, `Files.copy`), so an added or
removed resource reruns only the tests that looked for it or listed its directory.

## Parent PRD

[00](00-prd-local-first-test-records.md)

## What to build

Replace issue 04's "any resource edit runs everything" with recorded reads.

- The agent records the classpath resources each test class loads. Reads made while a context starts
  count for every test class that shares that context.
- Test classes that open files by path are found statically and treated as reading every resource.
  That covers constant strings pointing into resource directories, `file:` locations, and file-system
  APIs used with workspace-relative paths.
- An edited resource reruns the tests that read it plus those path readers. An edited resource that no
  test read selects nothing.
- Build inputs and `generated` inputs keep their existing rules.

## Acceptance criteria

- [ ] Editing a JSON fixture reruns only the tests that loaded it.
- [ ] Editing test configuration read at context startup reruns every test sharing that context.
- [ ] A test that reads a fixture by file path reruns on any resource edit.
- [ ] New oracle scenarios pass with zero missed failures.

## Blocked by

- 04

## User stories addressed

- User stories 3, 21
