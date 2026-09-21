#!/usr/bin/env bash
set -euo pipefail

tool=$1
workspace=$(mktemp -d)
selection=$(mktemp)
trap 'rm -rf "$workspace"; rm -f "$selection"' EXIT
mkdir -p "$workspace/src/main/java/example" "$workspace/src/test/java/example"
cat > "$workspace/src/main/java/example/Number.java" <<'EOF'
package example;
public class Number { public int next(int value) { return value + 1; } }
EOF
cat > "$workspace/src/test/java/example/NumberTest.java" <<'EOF'
package example;
import org.junit.Test;
import static org.junit.Assert.assertEquals;
public class NumberTest { @Test public void next() { assertEquals(2, new Number().next(1)); } }
EOF
printf 'target/\nbuild/\n.gradle/\n' > "$workspace/.gitignore"

if [[ $tool == maven ]]; then
  executable=${IMPACT_MAVEN:-mvn}
  cat > "$workspace/pom.xml" <<'EOF'
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion><groupId>example</groupId><artifactId>java-compat</artifactId><version>1</version>
  <properties><maven.compiler.source>8</maven.compiler.source><maven.compiler.target>8</maven.compiler.target></properties>
  <dependencies><dependency><groupId>junit</groupId><artifactId>junit</artifactId><version>4.13.2</version><scope>test</scope></dependency></dependencies>
  <build><plugins>
    <plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-compiler-plugin</artifactId><version>3.13.0</version></plugin>
    <plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-surefire-plugin</artifactId><version>3.5.2</version></plugin>
  </plugins></build>
</project>
EOF
  report="$workspace/target/surefire-reports/TEST-example.NumberTest.xml"
elif [[ $tool == gradle ]]; then
  executable=${IMPACT_GRADLE:-gradle}
  cat > "$workspace/build.gradle" <<'EOF'
plugins { id 'java' }
repositories { mavenCentral() }
dependencies { testImplementation 'junit:junit:4.13.2' }
EOF
  report="$workspace/build/test-results/test/TEST-example.NumberTest.xml"
else
  exit 2
fi

target/debug/sieve init --workspace "$workspace" --executable "$executable"
git -C "$workspace" init -q
git -C "$workspace" -c user.name=Test -c user.email=test@example.invalid add .
git -C "$workspace" -c user.name=Test -c user.email=test@example.invalid commit -qm baseline
sed 's/value + 1/value + 2/' "$workspace/src/main/java/example/Number.java" > "$workspace/Number.java.tmp"
mv "$workspace/Number.java.tmp" "$workspace/src/main/java/example/Number.java"
if target/debug/sieve run --workspace "$workspace" --base HEAD --executable "$executable" --output "$selection"; then
  echo 'Expected the selected test to fail' >&2
  exit 1
fi
jq -e '.mode == "MODULES" and .modules == ["."]' "$selection"
grep -q '<failure' "$report"
