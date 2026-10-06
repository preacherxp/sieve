---
status: accepted
---

# Select Maven test classes inside the build with a core extension

Class-level selection compiled the selected modules, chose test classes in Rust, and started the build again with Surefire and Failsafe excludes files. The second start made class-level runs slower than module-level runs on cheap suites (1.4 s to 2.2 s on `samples/selective-performance`), and ran every generate-sources plugin twice. Gradle now selects inside its one build: the init script's `impactSelect` task runs between compiling and testing (7.4 s to 4.6 s on the Gradle fixture). Maven has no such hook without help, so `sieve run` loads a small core extension with `-Dmaven.ext.class.path`. Right before a module's Surefire `test` or Failsafe `integration-test` mojo runs, the module and everything it depends on are compiled; the extension asks `sieve classes --module` for the module's excludes and points the mojo's `excludesFile` at them, after the POM's own excludes file. The selection rules stay in Rust; each module is decided from its own classes and those of the modules it depends on, which are the only ones its tests can reach.

## Considered Options

- The JUnit discovery filter of local mode (ADR 0002) in select-only mode: no new jar, but it narrows only JUnit Platform tests on Java 17+ test JVMs, and not with `useSystemClassLoader=false`; the excludes file works for every provider and JVM.
- An extension that writes an excludes file at a fixed relative path given on the command line: no reflection, but a build that does not load the extension would fail on the missing file, while a mojo the extension never touches simply runs every test.
- Keep two Maven starts: no code in Maven's JVM, at the cost measured above.

ADR 0002 rejected a core extension for local mode because plain `mvn` and IDE runs had to select too; static selection only ever runs through `sieve run`.

## Consequences

- One Maven start for class-level runs. A changed `generated` input still compiles first, because its sources are compared with the merge base's before any module is decided; so does a build that loads its own extensions through `maven.ext.class.path`.
- The extension depends on `MojoExecutionListener` and on the mojos' `excludesFile` field. When either is missing, or anything else goes wrong, the module runs every test and the build log says so.
- `--output` holds the module selection while the build runs and the refined one when it ends.
