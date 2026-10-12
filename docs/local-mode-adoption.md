# Adopting local mode on a service

Local mode gives developers on a single-module Maven or Gradle service short feedback loops: after
an edit, `sieve run --records` runs only the test classes whose recorded behavior the edit can
change, and it starts no build at all when nothing changed since the last green run. This
guide takes a service from zero to measured. The mechanics are described in the
[reference](reference.md#local-mode-test-records).

## Check the prerequisites

- One Maven or Gradle module (`modules: {".": []}`), JUnit Platform tests, and a test JVM
  on Java 17 or newer. On an older test JVM the agent stays inactive and every test runs.
- Surefire and Failsafe on their default class loader (`useSystemClassLoader` not set to
  `false`). Concurrent classes are supported: overlapping classes share evidence and
  rerun together when that evidence changes; nonoverlapping groups retain precision.
- A `sieve` with the agent: the release binary from `install.sh` (see the README), or
  `cargo install --git https://github.com/preacherxp/sieve --locked` on a machine whose
  `JAVA_HOME` (or `SIEVE_JAVA_HOME`) points to JDK 17+.

## Switch it on

Local mode is opt-in. Use `sieve run --records` to try it without configuration on a
single-module Maven project. For Gradle or shared configuration, run `sieve init` once
and review its generated configuration and any Maven POM edits. Keep `"records": true`
out of the committed configuration so normal CI commands retain static selection.
Add local-mode inputs to the generated `impact.json` without replacing its module graph
or fingerprint, for example:

```json
{
  "generated": ["src/main/resources/openapi/**"],
  "record_env": ["SPRING_PROFILES_ACTIVE"]
}
```

List the sources of generated code, such as OpenAPI specifications, under `generated`: an
edit to them then cleans the build first, so that no stale generated class survives.
List environment variables that affect tests under `record_env`; changing any of them
invalidates the records. Invocation arguments and stable JVM system properties also invalidate
records; `record_ignore_properties` excludes system properties that change with every
commit, and jgitver's `jgitver.*` properties are always excluded. jgitver also changes the
project version on every commit, which reruns every test reading a filtered resource that
embeds it, such as a Spring `application.yml`, so local runs skip jgitver by
default (`--without jgitver` keeps it). A configured plain `sieve run` without `"records": true` uses static selection
and runs the full suite when no base is supplied. Class selection remains a separate
opt-in through `"class_level": true`.
Commit the reviewed `impact.json` and any POM changes from `init`; `.sieve/` ignores
itself in Git, so records stay on the developer's machine.

The code must compile: Maven compiles every test before any runs. Then work as usual:

```bash
sieve run --records                      # first run records every test; later runs select
sieve run --records -- -Dtest=MyTest      # explicit tests always run (Gradle: -- --tests MyTest)
JDK_JAVA_OPTIONS="$(sieve env)" mvn verify   # the same selection, plain Maven
```

On Gradle, `sieve run --records` keeps the Gradle daemon, never cleans, and runs every `Test` task
by default instead of `check`, so linters do not run on every edit; `sieve run --records -- check`
runs them. Plain `./gradlew` runs are unaffected and keep no records.

`--output /tmp/selection.json` explains every test class: which method, resource, or wiring
change made it run, or that its record was unchanged.

The unchanged-run shortcut hashes file contents, so preserving a file's size and
modification time cannot hide an edit. Such content edits trigger a clean build to replace
stale compiled classes. Local CLI runs hold a workspace execution lock through the build
and reporting. Plain Maven callbacks defer selection and recording while a wrapper owns
that lock. Ordinary Maven and IDE builds can still modify native build outputs; Sieve
checks those outputs before skipping a build and rejects records for classes rewritten
while tests ran.

The build keeps native `verify` packaging by default. Startup shortcuts are explicit:
`--with reuse,mvnd,repackage` enables container reuse, using the Maven daemon, and skipping
Spring Boot repackaging respectively; skipping jgitver is already the default. Enable only the ones
that fit the service and measure them separately.

## Measure before relying on it

Write an edit catalog of the team's typical changes (a listener body, a service body, a
controller, a new field, configuration, a new test, a POM bump, an API specification) and
time it against the native build. Keep catalogs and results of internal services outside
the repository. For the catalog, copy `impact.json` to `../private/records.json` and
set `"records": true` in that evaluation-only copy:

```bash
sieve catalog --workspace . --catalog ../private/catalog.json \
  --config ../private/records.json --output ../private/records --plant
sieve replay --workspace . --walk --plant --commits 20 --output ../private/walk.json
```

The catalog weights edit kinds by the service's own history (`sieve classify`), and
`--plant` checks that a bug planted in each edited method fails the selected run whenever it
fails the full run. The walk replays recent commits in one working tree. Any missed failure
makes both commands exit with 1.

`replay` also writes a readable `../private/walk.json.md` report beside the JSON output.
Use its measured wall times and review inconclusive comparisons and differing failures,
including selected-only failures that may leave replay's exit code at zero.
The normal run summary's historical test durations estimate serial test time avoided;
they do not establish wall-clock savings, especially with parallel tests or shared setup.
For CI evaluation, use static module/class selection and the
[pilot checklist](reference.md#pilot-checklist), keeping full-suite validation.

## Make the tests cheap and independent

Once only a few test classes run per edit, the fixed cost per run dominates: starting
containers and Spring contexts. Records also assume that a test does not depend on what an
earlier test left behind. The usual fixes, in order of payoff:

1. Start each container once per JVM (a static singleton or a shared base class) instead
   of per test class, and declare it `withReuse(true)`. Opt into `--with reuse` to set
   `TESTCONTAINERS_REUSE_ENABLE=true`, so reused containers survive between runs.
2. Remove `@DirtiesContext` from shared base classes; reset state in the test instead.
3. Give each test its own data: unique topic names, consumer group ids, and document ids,
   and clean collections in `@AfterEach`.
4. Merge differing `@MockitoBean` sets into a few shared test configurations, so that Spring
   caches a handful of contexts rather than one per test class.

Run the full suite in both test-class orders before and after each change.

## Known limits

Records cover observed execution paths and assume independent tests. Declare environment
variables under `record_env`; undeclared variables, external services, floating Docker
image tags, and dependency jars that change without a POM change remain outside that
evidence. Lazily created beans count only for the test class that first used them. Keep
the full suite in CI.
