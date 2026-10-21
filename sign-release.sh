#!/bin/bash
# Detach-sign a draft release's assets, upload the signatures, publish the release.
set -euo pipefail

cd "$(dirname "$0")"

DRY_RUN=0
TAG=""
while (( $# )); do
    case "$1" in
        -n|--dry-run) DRY_RUN=1 ;;
        -h|--help)
            echo "usage: ${0##*/} [-n|--dry-run] [TAG]"
            exit 0
            ;;
        -*) echo "unknown option: $1" >&2; exit 1 ;;
        *) TAG="$1" ;;
    esac
    shift
done

TAG="${TAG:-v$(grep -Po '^version = "\K[^"]+' Cargo.toml)}"

# An unset key exits git non-zero, which set -e would turn into a silent exit
KEY=$(git config user.signingkey || true)
if [[ -z "$KEY" ]]; then
    echo "git config user.signingkey is unset: refusing to guess a signing key" >&2
    exit 1
fi

WORKDIR="scratch/release-sign-$TAG"
rm -rf "$WORKDIR"
mkdir -p "$WORKDIR"

gh release download "$TAG" --dir "$WORKDIR"

# Signatures from an earlier run of this script are replaced, never re-signed
rm -f "$WORKDIR"/*.asc

shopt -s nullglob
assets=("$WORKDIR"/*)
if (( ${#assets[@]} == 0 )); then
    echo "$TAG has no assets to sign" >&2
    exit 1
fi

for asset in "${assets[@]}"; do
    gpg --local-user "$KEY" --armor --detach-sign --yes --output "$asset.asc" "$asset"
    gpg --verify "$asset.asc" "$asset"
done

if (( DRY_RUN )); then
    echo "dry run: signed ${#assets[@]} assets of $TAG with $KEY, nothing uploaded, release left as-is"
    echo "signatures left in $WORKDIR for inspection:"
    printf '  %s\n' "$WORKDIR"/*.asc
    exit 0
fi

gh release upload "$TAG" "$WORKDIR"/*.asc --clobber

# CI leaves the release a draft so it goes public only once signed. Assets of a
# published release cannot be added or replaced when release immutability is on.
gh release edit "$TAG" --draft=false

rm -rf "$WORKDIR"
echo "Signed ${#assets[@]} assets of $TAG with $KEY"
echo "Remove a signature with: gh release delete-asset $TAG <name>.asc"
