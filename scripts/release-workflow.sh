#!/usr/bin/env bash
set -euo pipefail

valid_digest() {
  [[ "$1" =~ ^sha256:[0-9a-f]{64}$ ]]
}

case "${1:?Expected release, inspect, digest, or publish}" in
  release)
    response=$(mktemp)
    trap 'rm -f "$response"' EXIT
    endpoint="repos/${GH_REPO:?}/releases/tags/${RELEASE_TAG:?}"
    if gh api --include "$endpoint" > "$response"; then
      echo "Reusing release $RELEASE_TAG"
    elif grep -Eq '^HTTP/[^ ]+ 404([[:space:]]|$)' "$response"; then
      body=$(sed 's/^/- /' "${COMMITS_FILE:-/tmp/commits.txt}")
      # A concurrent run may have created the release after our lookup.
      if ! gh release create "$RELEASE_TAG" --title "Bolly $RELEASE_TAG" --notes "$body" --draft; then
        gh api "$endpoint" > /dev/null
      fi
    else
      echo 'Cannot determine whether the release exists' >&2
      exit 1
    fi
    ;;
  publish)
    # The workflow holds ghcr-latest for this entire publication/promotion sequence.
    if ! valid_digest "${IMAGE_DIGEST:-}"; then
      echo 'Missing or invalid image digest' >&2
      exit 1
    fi
    release_id=$(gh api "repos/${GH_REPO:?}/releases/tags/${RELEASE_TAG:?}" --jq .id)
    # Reuse the release; let GitHub select latest instead of forcing an older rerun.
    gh api --method PATCH "repos/${GH_REPO}/releases/${release_id}" \
      -F draft=false -f make_latest=legacy
    latest_tag=$(gh api "repos/${GH_REPO}/releases/latest" --jq .tag_name)
    if [ "$latest_tag" != "$RELEASE_TAG" ]; then
      echo "Skipping latest promotion: GitHub's latest release is $latest_tag."
      exit 0
    fi
    docker buildx imagetools create \
      --tag ghcr.io/triangle-int/bolly:latest \
      "ghcr.io/triangle-int/bolly@${IMAGE_DIGEST}"
    ;;
  inspect)
    ref="ghcr.io/triangle-int/bolly:${RELEASE_TAG:?}"
    error=$(mktemp)
    trap 'rm -f "$error"' EXIT
    if digest=$(docker buildx imagetools inspect "$ref" --format '{{.Manifest.Digest}}' 2> "$error"); then
      if ! valid_digest "$digest"; then
        echo "Cannot determine existing digest for $ref" >&2
        exit 1
      fi
      printf 'absent=false\ndigest=%s\n' "$digest" >> "${GITHUB_OUTPUT:?}"
    # Only an explicit missing manifest permits a push. Auth/network errors fail closed.
    elif [ "$(cat "$error")" = "ERROR: $ref: not found" ]; then
      echo 'absent=true' >> "${GITHUB_OUTPUT:?}"
    else
      cat "$error" >&2
      exit 1
    fi
    ;;
  digest)
    if ! valid_digest "${IMAGE_DIGEST:-}"; then
      echo 'Missing or invalid image digest' >&2
      exit 1
    fi
    printf 'digest=%s\n' "$IMAGE_DIGEST" >> "${GITHUB_OUTPUT:?}"
    ;;
  *) exit 2 ;;
esac
