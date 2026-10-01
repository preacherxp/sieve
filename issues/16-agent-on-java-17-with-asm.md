# 16 Agent on Java 17+ with vendored ASM

Type: AFK

Status (2026-10-01): implemented. ASM 9.10.1 core vendored in `agent/asm` as `sieve.agent.asm`; `Transformer` prepends probes without recomputing frames (only max stack). `build.rs` needs JDK 17+. `tests/records.rs` passes on a Java 21 test JVM; Java 17 and 25 test JVMs not run yet.

## Parent PRD

[15](15-prd-local-mode-for-gradle-and-java-17.md)

## What to build

Replace the Class-File API transformer, the agent's only Java 24 class, with one built on a vendored
copy of ASM, so records and selection work on Java 17+ test JVMs (ADR 0003).

- Vendor the ASM core sources at a pinned version, with its BSD-3-Clause license, under `agent/`.
  `build.rs` compiles them into `sieve.agent.asm` together with the agent, with `--release 17`.
- Port `Transformer` to ASM with the same probes: method-entry probes on project classes and file
  probes on the JDK file APIs. Frame computation resolves class hierarchies through the class loader,
  as the current transformer does.
- `Boot` activates the agent on Java 17+. A class file version the vendored ASM cannot read leaves
  that class uninstrumented and marks the run unrecordable, so nothing is skipped on partial evidence.
- `build.rs` accepts a JDK 17+ `javac`; README prerequisites and the local-mode guide change with it.

## Acceptance criteria

- [ ] `tests/records.rs` passes unchanged on Java 17, 21, and 25 test JVMs.
- [ ] A project with its own ASM version on the test class path is recorded correctly, and both ASM
      copies load without conflict.
- [ ] A test JVM newer than the vendored ASM supports runs every test and writes no records.
- [ ] Building Sieve with JDK 17 produces a binary with the agent; `--no-default-features` still
      builds without it.

## Blocked by

None - can start immediately.

## User stories addressed

- User stories 1, 8, 10
