#!/usr/bin/env bash
# Sets up Sieve's local mode (test records) on a single-module Maven service.
#
# Usage: scripts/setup-local-mode.sh SERVICE_DIR [--skip-install] [--no-run]
#   --skip-install  use the `sieve` already on PATH instead of installing this checkout
#   --no-run        configure only; skip the first recording run
set -euo pipefail

usage() { sed -n '2,7p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }
fail() { echo "error: $*" >&2; exit 1; }
warn() { echo "warning: $*" >&2; }

[ $# -ge 1 ] || usage
service="" install=1 run=1
for arg in "$@"; do
  case "$arg" in
    --skip-install) install=0 ;;
    --no-run) run=0 ;;
    -h|--help) usage ;;
    -*) fail "unknown option $arg" ;;
    *) service="$arg" ;;
  esac
done
[ -n "$service" ] || usage
service="$(cd "$service" && pwd)"
sieve_root="$(cd "$(dirname "$0")/.." && pwd)"

echo "== Checking $service"
[ -f "$service/pom.xml" ] || fail "no pom.xml in $service"
grep -q "<modules>" "$service/pom.xml" && fail "multi-module builds are not supported by local mode"
command -v python3 >/dev/null || fail "python3 is needed to edit impact.json"

java_bin="${JAVA_HOME:+$JAVA_HOME/bin/}java"
version="$("$java_bin" -XshowSettings:properties -version 2>&1 | awk -F'= ' '/java.specification.version/ {print $2}')"
[ -n "$version" ] || fail "cannot run $java_bin"
[ "${version%%.*}" -ge 17 ] 2>/dev/null || fail "test records need a Java 17+ test JVM (JAVA_HOME has $version)"
echo "Java $version"

if [ -x "$service/mvnw" ]; then maven="$service/mvnw"; else maven="mvn"; fi
command -v "$maven" >/dev/null || fail "Maven not found"

if docker info >/dev/null 2>&1; then
  echo "Docker is running"
else
  warn "Docker is not running; Testcontainers tests will fail"
fi

if grep -q "<useSystemClassLoader>false" "$service/pom.xml"; then
  warn "Surefire/Failsafe useSystemClassLoader=false is not supported; the agent stays inactive"
fi
if grep -rqs "junit.jupiter.execution.parallel.enabled *= *true" "$service/src/test/resources"; then
  warn "JUnit parallel execution is enabled; parallel runs keep no records"
fi

if [ "$install" = 1 ]; then
  echo "== Installing sieve from $sieve_root"
  command -v cargo >/dev/null || fail "cargo not found; install Rust 1.92+ or pass --skip-install"
  cargo install --path "$sieve_root" --locked --force
fi
command -v sieve >/dev/null || fail "sieve is not on PATH (cargo installs to ~/.cargo/bin)"

echo "== Configuring impact.json"
python3 - "$service" <<'PY'
import json, os, re, sys

service = sys.argv[1]
path = os.path.join(service, "impact.json")
config = json.load(open(path)) if os.path.exists(path) else {"tool": "maven", "modules": {".": []}}
if config.get("tool") != "maven" or list(config.get("modules", {})) != ["."]:
    sys.exit("impact.json does not describe a single-module Maven project")
config["records"] = True
# Sources of generated code: an edit to them cleans the build first.
pom = open(os.path.join(service, "pom.xml")).read()
generated = set(config.get("generated", []))
for spec in re.findall(r"<inputSpec>\s*([^<]+?)\s*</inputSpec>", pom):
    spec = spec.replace("${project.basedir}/", "").replace("${basedir}/", "")
    if not spec.startswith("$") and "/" in spec:
        generated.add(os.path.dirname(spec) + "/**")
if generated:
    config["generated"] = sorted(generated)
with open(path, "w") as out:
    json.dump(config, out, indent=2)
    out.write("\n")
print(json.dumps(config, indent=2))
PY

if [ "$run" = 1 ]; then
  echo "== First run: every test runs once and leaves a record"
  sieve run --workspace "$service" --executable "$maven" --output "$service/.sieve/first-run.json"
fi

cat <<EOF

Local mode is set up. From $service, before and after every edit:
  sieve run                          # only tests an edit can affect
  sieve run --output /tmp/sel.json   # why each test ran or was skipped
  sieve run --base origin/main       # static fallback after a green run with the same invocation
  sieve run --full                   # everything, still recording
  JDK_JAVA_OPTIONS="\$(sieve env)" mvn verify   # the same selection, plain Maven
Records live in .sieve/, which Git ignores. Commit impact.json if the team should share it.
EOF
