#!/usr/bin/env bash
set -euo pipefail

release_id() {
  gh release view "${RELEASE_TAG:?}" \
    --repo "${GH_REPO:?}" \
    --json databaseId \
    --jq .databaseId
}

case "${1:?Expected release or publish}" in
  release)
    error=$(mktemp)
    trap 'rm -f "$error"' EXIT
    if release_id > /dev/null 2> "$error"; then
      echo "Reusing release $RELEASE_TAG"
    elif grep -Fxq 'release not found' "$error"; then
      body=$(sed 's/^/- /' "${COMMITS_FILE:-/tmp/commits.txt}")
      # A concurrent run may have created the release after our lookup.
      if ! gh release create "$RELEASE_TAG" \
        --repo "$GH_REPO" \
        --title "Nolune $RELEASE_TAG" \
        --notes "$body" \
        --draft; then
        release_id > /dev/null
      fi
    else
      cat "$error" >&2
      echo 'Cannot determine whether the release exists' >&2
      exit 1
    fi
    ;;
  publish)
    id=$(release_id)
    gh api --method PATCH "repos/${GH_REPO}/releases/${id}" \
      -F draft=false -f make_latest=legacy
    ;;
  *) exit 2 ;;
esac
