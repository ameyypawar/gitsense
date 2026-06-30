#!/usr/bin/env bash
# gitsense demo runner — invoked by asciinema rec
# Runs demo/driver.py from the repo root.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
exec python3 "$REPO_ROOT/demo/driver.py"
