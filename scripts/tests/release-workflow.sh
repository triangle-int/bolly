#!/usr/bin/env bash
# Deterministic tests: no network, GitHub, registry, or Docker daemon access.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export PATH="$tmp:$PATH"
export GITHUB_OUTPUT="$tmp/output" CALLS="$tmp/calls" COMMITS_FILE="$tmp/commits"
export GH_REPO=triangle-int/bolly RELEASE_TAG=v1.2.3
EXPECTED_DIGEST="sha256:$(printf '%064d' 1)"
export EXPECTED_DIGEST
printf 'abc123 Test change\n' > "$COMMITS_FILE"
cat > "$tmp/docker" <<'MOCK'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >> "$CALLS"
[[ "$*" == "buildx imagetools inspect ghcr.io/triangle-int/bolly:v1.2.3 --format {{.Manifest.Digest}}" ]]
case "$SCENARIO" in
  existing) echo "$EXPECTED_DIGEST" ;;
  empty) : ;;
  invalid) echo sha256:bad ;;
  absent) echo 'ERROR: ghcr.io/triangle-int/bolly:v1.2.3: not found' >&2; exit 1 ;;
  auth) echo 'ERROR: unauthorized' >&2; exit 1 ;;
  network) echo 'ERROR: connection timed out' >&2; exit 1 ;;
  unknown) echo 'ERROR: unexpected not found response' >&2; exit 1 ;;
  *) exit 99 ;;
esac
MOCK
cat > "$tmp/gh" <<'MOCK'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >> "$CALLS"
case "$1 $2" in
  'api --include')
    case "$SCENARIO" in
      draft|published) printf 'HTTP/2.0 200 OK\n\n{"draft":%s}\n' "$([ "$SCENARIO" = draft ] && echo true || echo false)" ;;
      absent|race|create_error) printf 'HTTP/2.0 404 Not Found\n'; exit 1 ;;
      auth) printf 'HTTP/2.0 403 Forbidden\n'; exit 1 ;;
      network) exit 1 ;;
      *) exit 99 ;;
    esac ;;
  'release create')
    [[ "$*" == 'release create v1.2.3 --title Bolly v1.2.3 --notes - abc123 Test change --draft' ]]
    [[ "$SCENARIO" == absent ]] ;;
  'api repos/triangle-int/bolly/releases/tags/v1.2.3')
    [[ "$SCENARIO" == race ]] ;;
  *) exit 99 ;;
esac
MOCK
chmod +x "$tmp/docker" "$tmp/gh"
run() {
  local mode scenario expected status
  mode=$1
  scenario=$2
  expected=$3
  status=0
  export SCENARIO=$scenario
  : > "$GITHUB_OUTPUT"
  : > "$CALLS"
  bash "$root/scripts/release-workflow.sh" "$mode" > "$tmp/log" 2>&1 || status=$?
  if { [ "$expected" = success ] && [ "$status" -ne 0 ]; } ||
     { [ "$expected" = failure ] && [ "$status" -eq 0 ]; }; then
    cat "$tmp/log" >&2
    echo "FAIL: $mode $scenario ($status)" >&2
    exit 1
  fi
}
run inspect absent success
[[ $(cat "$GITHUB_OUTPUT") == absent=true ]]
run inspect existing success
[[ $(cat "$GITHUB_OUTPUT") == "$(printf 'absent=false\ndigest=%s' "$EXPECTED_DIGEST")" ]]
for scenario in empty invalid auth network unknown; do
  run inspect "$scenario" failure
  [[ ! -s "$GITHUB_OUTPUT" ]]
done
# Both reused and newly built digests pass through the same output validator.
for value in "$EXPECTED_DIGEST" "sha256:$(printf '%064d' 2)"; do
  export IMAGE_DIGEST=$value
  run digest valid success
  [[ $(cat "$GITHUB_OUTPUT") == "digest=$value" ]]
done
for value in '' sha256:bad; do
  export IMAGE_DIGEST=$value
  run digest invalid failure
  [[ ! -s "$GITHUB_OUTPUT" ]]
done
for scenario in draft published; do
  run release "$scenario" success
  [[ $(wc -l < "$CALLS") -eq 1 ]]
  if grep -q 'release create' "$CALLS"; then
    echo "FAIL: reused release attempted creation" >&2
    exit 1
  fi
done
run release absent success
[[ $(grep -c '^release create ' "$CALLS") -eq 1 ]]
run release race success
[[ $(wc -l < "$CALLS") -eq 3 ]]
run release create_error failure
for scenario in auth network; do
  run release "$scenario" failure
  if grep -q 'release create' "$CALLS"; then
    echo "FAIL: lookup failure attempted release creation" >&2
    exit 1
  fi
done
# Check the scheduler boundary as well as the shell sequence: neither publication
# nor promotion may escape the single job holding the cross-version lock.
python3 - "$root/.github/workflows/release.yml" <<'CHECK'
import re
import sys
from pathlib import Path
workflow = Path(sys.argv[1]).read_text()
jobs = dict(re.findall(r"^  ([\w-]+):\n(.*?)(?=^  [\w-]+:|\Z)", workflow, re.M | re.S))
job = jobs["publish"]
for required in (
    "    needs: [server, desktop, docker]",
    "    concurrency:\n      group: ghcr-latest\n      cancel-in-progress: false",
    "    permissions:\n      contents: write\n      packages: write",
    "          IMAGE_DIGEST: ${{ needs.docker.outputs.digest }}",
    "        run: bash scripts/release-workflow.sh publish",
):
    assert required in job, required
assert workflow.count("scripts/release-workflow.sh publish") == 1
assert "promote-latest" not in jobs
assert "make_latest" not in workflow
assert "imagetools create" not in workflow
CHECK

# Stateful mocks model GitHub's latest selection and the registry's latest digest.
cat > "$tmp/gh" <<'MOCK'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >> "$CALLS"
case "$*" in
  "api repos/$GH_REPO/releases/tags/$RELEASE_TAG --jq .id")
    [[ "$SCENARIO" != lookup_error ]] || exit 1
    echo "$RELEASE_TAG" ;;
  "api --method PATCH repos/$GH_REPO/releases/$RELEASE_TAG -F draft=false -f make_latest=legacy")
    [[ "$SCENARIO" != publish_error ]] || exit 1
    if [[ ! -s "$LATEST_TAG" || "$RELEASE_TAG" > "$(cat "$LATEST_TAG")" ]]; then
      echo "$RELEASE_TAG" > "$LATEST_TAG"
    fi ;;
  "api repos/$GH_REPO/releases/latest --jq .tag_name")
    [[ "$SCENARIO" != latest_error ]] || exit 1
    cat "$LATEST_TAG" ;;
  *) exit 99 ;;
esac
MOCK
cat > "$tmp/docker" <<'MOCK'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >> "$CALLS"
[[ "$*" == "buildx imagetools create --tag ghcr.io/triangle-int/bolly:latest ghcr.io/triangle-int/bolly@$IMAGE_DIGEST" ]]
[[ "$SCENARIO" != promotion_error ]] || exit 1
echo "$IMAGE_DIGEST" > "$LATEST_DIGEST"
MOCK
export LATEST_TAG="$tmp/latest-tag" LATEST_DIGEST="$tmp/latest-digest"
old_digest=$EXPECTED_DIGEST
new_digest="sha256:$(printf '%064d' 2)"
# Job concurrency allows either order, but no publication/promotion interleaving.
# Include an old rerun after the newer release in each ordering.
for order in 'v1.2.3 v1.2.4 v1.2.3' 'v1.2.4 v1.2.3 v1.2.3'; do
  : > "$LATEST_TAG"
  : > "$LATEST_DIGEST"
  for tag in $order; do
    export RELEASE_TAG=$tag IMAGE_DIGEST=$old_digest
    if [[ "$tag" == v1.2.4 ]]; then export IMAGE_DIGEST=$new_digest; fi
    run publish ordering success
    expected="$(printf '%s\n' \
      "api repos/$GH_REPO/releases/tags/$RELEASE_TAG --jq .id" \
      "api --method PATCH repos/$GH_REPO/releases/$RELEASE_TAG -F draft=false -f make_latest=legacy" \
      "api repos/$GH_REPO/releases/latest --jq .tag_name")"
    if [[ "$(cat "$LATEST_TAG")" == "$tag" ]]; then
      expected="$expected
buildx imagetools create --tag ghcr.io/triangle-int/bolly:latest ghcr.io/triangle-int/bolly@$IMAGE_DIGEST"
    fi
    [[ "$(cat "$CALLS")" == "$expected" ]]
    if [[ "$(cat "$LATEST_TAG")" == v1.2.4 ]]; then
      [[ "$(cat "$LATEST_DIGEST")" == "$new_digest" ]]
    fi
  done
  [[ "$(cat "$LATEST_DIGEST")" == "$new_digest" ]]
done
export RELEASE_TAG=v1.2.4 IMAGE_DIGEST=$new_digest
for scenario in lookup_error publish_error latest_error promotion_error; do
  echo "$old_digest" > "$LATEST_DIGEST"
  run publish "$scenario" failure
  [[ "$(cat "$LATEST_DIGEST")" == "$old_digest" ]]
  if [[ "$scenario" != promotion_error ]]; then
    if grep -q '^buildx imagetools create ' "$CALLS"; then
      echo "FAIL: $scenario reached latest promotion" >&2
      exit 1
    fi
  fi
done
for value in '' sha256:bad; do
  export IMAGE_DIGEST=$value
  run publish invalid failure
  [[ ! -s "$CALLS" ]]
done
printf 'All release workflow shell tests passed.\n'
