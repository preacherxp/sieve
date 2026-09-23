#!/usr/bin/env bash
# Runs the full suite, then class-level selections for typical edits, on a throwaway
# Git copy of this sample. Needs Java 17+, Maven, and sieve (override with SIEVE/MVN).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
sieve=${SIEVE:-sieve}
mvn=${MVN:-mvn}
temp=$(mktemp -d)
trap 'rm -rf "$temp"' EXIT
work="$temp/bookstore"
mkdir "$work"
tar -C "$here" --exclude target -cf - . | tar -C "$work" -xf -
git -C "$work" init -q
git -C "$work" add .
git -C "$work" -c user.name=demo -c user.email=demo@example.invalid -c commit.gpgsign=false \
  commit -qm baseline

run() {
  local label=$1
  shift
  local start=$SECONDS
  if ! "$sieve" run --workspace "$work" --executable "$mvn" --output "$temp/selection.json" \
    "$@" -- -q >"$temp/build.log" 2>&1; then
    cat "$temp/build.log"
    exit 1
  fi
  local mode classes
  mode=$(sed -n 's/^  "mode": "\(.*\)",$/\1/p' "$temp/selection.json")
  classes=$({ find "$work/target" -path '*-reports/TEST-*.xml' 2>/dev/null || true; } | wc -l | tr -d ' ')
  printf '%-36s %-8s %2s test classes %4ss\n' "$label" "$mode" "$classes" $((SECONDS - start))
}

edit() {
  local file=$1 from=$2 to=$3
  sed -i.bak "s|$from|$to|" "$work/$file" && rm "$work/$file.bak"
}

reset() {
  git -C "$work" checkout -q -- .
}

run "full suite" --full
edit src/main/java/bookstore/notify/EmailFormatter.java "Thanks for" "Thank you for"
run "edit notify/EmailFormatter" --base HEAD
reset
edit src/main/java/bookstore/pricing/BulkDiscount.java "quantity >= 10" "quantity > 9"
run "edit pricing/BulkDiscount" --base HEAD
reset
edit src/main/java/bookstore/report/SalesReport.java "revenue = new" "revenue =  new"
run "edit report/SalesReport" --base HEAD
reset
edit src/main/java/bookstore/catalog/Book.java "record Book" "record  Book"
run "edit catalog/Book (used everywhere)" --base HEAD
reset
printf '\nNotes.\n' >>"$work/README.md"
run "edit README.md" --base HEAD
