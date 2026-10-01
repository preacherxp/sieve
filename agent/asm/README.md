# Vendored ASM

The core package of [ASM](https://asm.ow2.io/) 9.10.1 (`org.ow2.asm:asm:9.10.1`, sources jar
SHA-1 `9f9a96aa17fd1113b218fa2dd6c4dc075192aa2e`), relocated from `org.objectweb.asm` to `sieve.agent.asm` so that it cannot clash
with an ASM on the test class path. Only the package name was changed. BSD-3-Clause; see
[LICENSE.txt](LICENSE.txt). ADR 0003 records why the agent uses ASM.

To upgrade: download the new `asm-<version>-sources.jar`, verify its `.sha1`, replace these
files with `org/objectweb/asm/*.java` rewritten by `sed 's/org\.objectweb\.asm/sieve.agent.asm/g'`,
and update the version and checksum here.
