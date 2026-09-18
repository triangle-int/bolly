#!/usr/bin/env bash
# Deterministic tests: no network or GitHub access.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export PATH="$tmp:$PATH"
export CALLS="$tmp/calls" COMMITS_FILE="$tmp/commits"
export GH_REPO=triangle-int/bolly RELEASE_TAG=v1.2.3
printf 'abc123 Test change\n' > "$COMMITS_FILE"

cat > "$tmp/gh" <<'MOCK'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >> "$CALLS"
case "$1 $2" in
  'release view')
    case "$SCENARIO" in
      draft|published|success|publish_error) echo 123 ;;
      race)
        if [[ $(wc -l < "$CALLS") -eq 1 ]]; then
          echo 'release not found' >&2
          exit 1
        fi
        echo 123 ;;
      absent|create_error) echo 'release not found' >&2; exit 1 ;;
      lookup_error) echo 'network unavailable' >&2; exit 1 ;;
      *) exit 99 ;;
    esac ;;
  'release create')
    [[ "$*" == 'release create v1.2.3 --repo triangle-int/bolly --title Bolly v1.2.3 --notes - abc123 Test change --draft' ]]
    [[ "$SCENARIO" == absent ]] ;;
  'api --method')
    [[ "$*" == 'api --method PATCH repos/triangle-int/bolly/releases/123 -F draft=false -f make_latest=legacy' ]]
    [[ "$SCENARIO" != publish_error ]] ;;
  *) exit 99 ;;
esac
MOCK
chmod +x "$tmp/gh"

run() {
  local mode scenario expected status
  mode=$1
  scenario=$2
  expected=$3
  status=0
  export SCENARIO=$scenario
  : > "$CALLS"
  bash "$root/scripts/release-workflow.sh" "$mode" > "$tmp/log" 2>&1 || status=$?
  if { [ "$expected" = success ] && [ "$status" -ne 0 ]; } ||
     { [ "$expected" = failure ] && [ "$status" -eq 0 ]; }; then
    cat "$tmp/log" >&2
    echo "FAIL: $mode $scenario ($status)" >&2
    exit 1
  fi
}

for scenario in draft published; do
  run release "$scenario" success
  [[ $(wc -l < "$CALLS") -eq 1 ]]
  if grep -q 'release create' "$CALLS"; then
    echo "FAIL: existing $scenario release triggered create" >&2
    exit 1
  fi
done
run release absent success
[[ $(grep -c '^release create ' "$CALLS") -eq 1 ]]
run release race success
[[ $(wc -l < "$CALLS") -eq 3 ]]
run release create_error failure
run release lookup_error failure
if grep -q 'release create' "$CALLS"; then
  echo 'FAIL: lookup error triggered create' >&2
  exit 1
fi

run publish success success
[[ $(cat "$CALLS") == "$(printf '%s\n' \
  'release view v1.2.3 --repo triangle-int/bolly --json databaseId --jq .databaseId' \
  'api --method PATCH repos/triangle-int/bolly/releases/123 -F draft=false -f make_latest=legacy')" ]]
run publish lookup_error failure
[[ $(wc -l < "$CALLS") -eq 1 ]]
run publish publish_error failure
[[ $(wc -l < "$CALLS") -eq 2 ]]

python3 - "$root/.github/workflows/release.yml" <<'CHECK'
import re
import sys
from pathlib import Path
workflow = Path(sys.argv[1]).read_text()
jobs = dict(re.findall(r"^  ([\w-]+):\n(.*?)(?=^  [\w-]+:|\Z)", workflow, re.M | re.S))
for required_job in ("create-release", "server", "desktop", "publish"):
    assert required_job in jobs
assert "docker" not in jobs
publish = jobs["publish"]
assert "    needs: [server, desktop]" in publish
assert "      contents: write" in publish
assert "packages: write" not in workflow
assert "docker" not in workflow.lower()
assert "ghcr" not in workflow.lower()
assert workflow.count("scripts/release-workflow.sh publish") == 1
CHECK

printf 'All release workflow shell tests passed.\n'
