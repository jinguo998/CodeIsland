#!/usr/bin/env bash
# Mirror the WSL checkout to a Windows-visible path for native builds.
#
# Usage:
#   desktop/scripts/sync-to-windows.sh [destination]
#
# Default destination: /mnt/c/dev/CodeIsland/desktop
# Override with CODEISLAND_WINDOWS_DEST or the first argument.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SOURCE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
DESTINATION="${1:-${CODEISLAND_WINDOWS_DEST:-/mnt/c/dev/CodeIsland/desktop}}"

if ! command -v rsync >/dev/null 2>&1; then
  echo "error: rsync is required (sudo apt install rsync)" >&2
  exit 1
fi

mkdir -p "$DESTINATION"

echo "Syncing $SOURCE_DIR -> $DESTINATION"
rsync -a --human-readable --itemize-changes \
  --exclude 'node_modules/' \
  --exclude 'dist/' \
  --exclude 'target/' \
  --exclude 'src-tauri/target/' \
  --exclude '.vscode/' \
  --exclude '*.log' \
  "$SOURCE_DIR/" "$DESTINATION/"

# The desktop Swift package builds and tests the macOS sources from this mirror.
for shared_dir in Sources/CodeIslandCore Tests/CodeIslandCoreTests; do
  mkdir -p "$DESTINATION/../$shared_dir"
  rsync -a "$SOURCE_DIR/../$shared_dir/" "$DESTINATION/../$shared_dir/"
done

# The desktop and macOS builds embed the same remote hook and OpenCode plugin.
REMOTE_HOOK_DIR="Sources/CodeIsland/Resources"
mkdir -p "$DESTINATION/../$REMOTE_HOOK_DIR"
rsync -a "$SOURCE_DIR/../$REMOTE_HOOK_DIR/codeisland-remote-hook.py" \
  "$SOURCE_DIR/../$REMOTE_HOOK_DIR/codeisland-opencode-remote.js" \
  "$DESTINATION/../$REMOTE_HOOK_DIR/"

cat <<EOF

Sync complete.

Next commands (Windows PowerShell or cmd.exe):
  cd C:\\dev\\CodeIsland\\desktop
  npm ci
  npm run tauri dev          # hot-reload desktop shell
  npm run tauri build        # production build + bundles

From WSL you can still run the frontend only:
  cd $SOURCE_DIR
  npm run dev                # Vite on http://localhost:1420

After editing Rust or config files, re-run this script before a Windows build.
EOF
