#!/bin/bash
# Wait for the CI run on one commit, and fail if it fails.
#
# Usage: ./watch-ci.sh [ref] [workflow]   (defaults: HEAD, build.yml)
#
# The run is selected by workflow AND commit SHA. Selecting with
# `--branch ... --limit 1` reads whatever ran most recently on the branch, which
# is not necessarily the commit being released. `gh run watch` also exits 0 on a
# failed run unless given --exit-status.
#
# A run does not exist the instant a push returns, so the id is polled for.

set -euo pipefail

POLL_TIMEOUT=180
POLL_INTERVAL=5

ref="${1:-HEAD}"
workflow="${2:-build.yml}"
# gh matches only the full SHA, and an annotated tag resolves to the tag object,
# which no run matches; the release tag is always annotated.
sha="$(git rev-parse "${ref}^{commit}")"

run_id=""
waited=0
while [ -z "$run_id" ]; do
	run_id="$(gh run list --workflow "$workflow" --commit "$sha" --limit 1 \
		--json databaseId --jq '.[0].databaseId // empty')"
	[ -n "$run_id" ] && break
	if [ "$waited" -ge "$POLL_TIMEOUT" ]; then
		echo "no $workflow run appeared for $ref ($sha) within ${POLL_TIMEOUT}s" >&2
		echo "push it first, or check that the workflow triggers on this ref" >&2
		exit 1
	fi
	sleep "$POLL_INTERVAL"
	waited=$((waited + POLL_INTERVAL))
done

echo "watching $workflow run $run_id for $ref ($sha)"
gh run watch --exit-status "$run_id"
