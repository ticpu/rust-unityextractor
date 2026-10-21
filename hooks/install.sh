#!/bin/bash
# Install git pre-commit hook

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GIT_DIR="$(git rev-parse --git-dir)" || {
	echo "Error: Not in a git repository"
	exit 1
}

# Handle worktrees: use common-dir if available (git 2.5+)
if [ -f "$GIT_DIR/commondir" ]; then
	COMMON_DIR="$(cat "$GIT_DIR/commondir")"
	# Make absolute if relative
	[[ "$COMMON_DIR" != /* ]] && COMMON_DIR="$GIT_DIR/$COMMON_DIR"
	GIT_HOOKS_DIR="$COMMON_DIR/hooks"
else
	GIT_HOOKS_DIR="$GIT_DIR/hooks"
fi

mkdir -p "$GIT_HOOKS_DIR"

echo "Installing pre-commit hook to $GIT_HOOKS_DIR..."
ln -sf "$(realpath --relative-to="$GIT_HOOKS_DIR" "$SCRIPT_DIR/pre-commit")" "$GIT_HOOKS_DIR/pre-commit"
echo "Done!"
