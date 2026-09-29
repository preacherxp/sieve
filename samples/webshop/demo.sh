#!/usr/bin/env bash
# Builds a Git history of typical changes on a throwaway copy of this sample, including
# commits that break tests and the commits that fix them, then replays it with
# `sieve replay --run`: every commit runs the full suite and the selection, and the
# report counts failures the selection missed. Needs Java 17+, Maven, jq, and sieve
# (override with SIEVE/MVN). Takes about 20 minutes.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
sieve=${SIEVE:-sieve}
mvn=${MVN:-mvn}
output=${OUTPUT:-}
temp=$(mktemp -d)
trap 'rm -rf "$temp"' EXIT
work="$temp/webshop"
mkdir "$work"
tar -C "$here" --exclude target -cf - . | tar -C "$work" -xf -
git() {
  command git -C "$work" -c user.name=demo -c user.email=demo@example.invalid -c commit.gpgsign=false "$@"
}
git init -q
git add .
git commit -qm baseline

# change FILE FROM TO MESSAGE: replaces the first match on each line and commits.
change() {
  local file=$1 from=$2 to=$3 message=$4
  grep -qF -- "$from" "$work/$file" || { echo "$file: '$from' not found" >&2; exit 1; }
  FROM=$from TO=$to perl -pi -e 's/\Q$ENV{FROM}\E/$ENV{TO}/' "$work/$file"
  git commit -qam "$message"
}
fix() {
  git revert --no-edit HEAD >/dev/null
}

main=src/main/java/shop
change README.md "# Webshop sample" "# Webshop example" "docs: retitle README"
change catalog/$main/catalog/ProductRepository.java '"merch", Money.of(1250)' '"merch", Money.of(1300)' \
  "catalog: raise mug price (breaks the end-to-end order total)"
fix
change pricing/$main/pricing/BulkDiscount.java "THRESHOLD = 10" "THRESHOLD = 11" "pricing: bulk discount from 11 items"
fix
change pricing/$main/pricing/CategoryPromotion.java "percentByCategory" "percentOff" "pricing: rename promotion field"
change inventory/$main/inventory/StockLedger.java "units < quantity" "units <= quantity" \
  "inventory: off-by-one in stock check"
fix
change notifications/$main/notifications/EmailRenderer.java "Thanks for" "Thank you for" "notifications: reword email"
change orders/$main/orders/clients/Resilience.java "backoff(2," "backoff(1," "orders: retry downstream calls once"
fix
change common/$main/common/web/CorrelationId.java '"X-Correlation-Id"' '"X-Request-Id"' "common: rename correlation header"
fix
change orders/$main/orders/OrderService.java ".doOnNext(saved -> events.publish(" \
  ".doOnNext(saved -> java.util.Objects.requireNonNull(" "orders: stop publishing order events"
fix
change common/$main/common/Money.java '"%s %d.%02d"' '"%s%d.%02d"' "common: compact money format"
fix
change catalog/src/main/resources/application.properties "server.port=8081" \
  "server.port=8081
spring.main.banner-mode=off" "catalog: disable banner"
change orders/src/test/java/shop/orders/OrderFlowIT.java "Duration.ofSeconds(5)" "Duration.ofSeconds(10)" \
  "orders: longer event stream timeout in test"
commits=$(git rev-list --count HEAD)

report=${output:-$temp/replay.json}
# Failing tests must not stop the reactor, or later modules never report theirs.
"$sieve" replay --workspace "$work" --commits $((commits - 1)) --run --executable "$mvn" \
  --output "$report" -- -q -Dmaven.test.failure.ignore=true || true
jq -r '
  ["commit", "mode", "full s", "selected s", "full failed", "selected failed", "missed"],
  (.records | reverse[] | [.subject[0:52], .selection.mode, (.full.seconds | floor),
    (.selected.seconds | floor), (.full.failed | length), (.selected.failed | length), (.missed | length)])
  | @tsv' "$report" | column -t -s $'\t'
jq '.summary' "$report"
jq -e '.summary.missed_failures == 0' "$report" >/dev/null
