---
status: proposed
---

# Select JavaScript and TypeScript tests through the runner's import graph

Sieve selected only JVM tests. Jest and Vitest already select by their own import graphs (`jest --changedSince`, `vitest --changed`), but neither falls back to the full suite when unsure: Jest finds no related test when only its configuration, `package.json`, or the lockfile changes, and Vitest reruns everything for `package.json`, its configuration, and its setup files, but not for the lockfile, `tsconfig`, or the files a setup file imports. Sieve therefore supports single-package Jest and Vitest projects without analysing JavaScript itself. It keeps its path classification, fallbacks, `--output`, and `replay`, asks the runner which test files relate to the changed script files inside the declared source roots, and runs exactly those through the runner's own command line.

## Decision details

- A script file that no test file reaches selects nothing, as an unreached class does in class-level selection. Deleted files, non-script files, and changes outside the source roots select every test, as deleted sources, resources, and build inputs do on the JVM.
- A snapshot file, `__snapshots__/<test file>.snap`, selects only its own test file. Sieve maps it itself, because Jest does and Vitest does not; the runners also disagree on imported CSS, so Sieve's rules, not the runner's view, decide non-script files.
- Runner-loaded files select every test. Sieve resolves them from the runner's configuration on every run, together with everything they import: no test imports a setup file, so the graph alone would skip every test for an edit to it.
- Jest is queried through its public command line (`--showConfig`, `--listTests --findRelatedTests --json`); whether a change reaches a runner-loaded file is the same query with those files as the only test files (`--testMatch`, or `--testRegex` when the configuration uses it, since Jest rejects both) and the whole workspace as `--roots`. Vitest's command line cannot list related files, so a small Node script embedded in the binary, like the agent jar, uses the `vitest/node` API (`createVitest`, `getRelevantTestSpecifications`). With `projects`, Vitest ignores an `include` override, so the setup-file walk calls the private `specifications.filterTestsBySource`, the walk `getRelevantTestSpecifications` itself uses.
- Workspaces come later. They need nested module paths, which Sieve lacks for Maven and Gradle as well.

## Considered Options

- Own import graph in Rust, as `classes.rs` does for bytecode: independent of the runner, but any difference from the runner's module resolution (tsconfig paths, package exports, bundler aliases) silently drops tests.
- Runtime records per test file, as local mode keeps on the JVM: sees computed imports, but repeats ADRs 0001–0005 for Node, and watch mode already serves the developer loop.
- Workspace packages only: the smallest change, but Nx, Turborepo, and `pnpm --filter` already do this.
- One runner start, with the helper running the selected tests: no second start, but flags after `--` would need translating, and reports would differ from the native run.

## Consequences

- Safety rests on one more assumption: tests reach project code only through literal imports or runner-loaded files. Scripts that tests spawn, workers started by path, script files read as fixtures, and imports with computed paths are invisible; source roots should leave such files out.
- A narrowed run starts the runner twice, once to query and once to run. The queries took under a second on a small probe project (Jest 30.5, Vitest 5.0); `replay` must show the cost on real suites.
- Selection depends on Jest's command line and Vitest's advanced and private API. A failed query is an analysis error and selects every test, so a Vitest release that removes the private walk costs speed, not safety.
- Jest exposes no list of the files its configuration imports; they count only through `--showConfig` paths.
- Paths passed to the runner must be canonical: Vitest matched nothing for `/tmp` paths that resolve to `/private/tmp`.
