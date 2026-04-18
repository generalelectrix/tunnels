#!/usr/bin/env bash
# Find dangling snapshot PNGs that no test references.
# Usage: ./scripts/check-snapshots.sh

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

# Collect snapshot names referenced in Rust test code.
# Handles both direct .snapshot("name") and helper functions that forward to it.
expected=$(grep -rhE '\.snapshot\("|_and_plot\("' --include='*.rs' \
    | sed 's/.*\.snapshot("\([^"]*\)".*/\1/; s/.*_and_plot("\([^"]*\)".*/\1/' \
    | sort -u)

# Collect snapshot PNGs on disk (excluding diff/old/new artifacts).
on_disk=$(find . -path '*/tests/snapshots/*.png' \
    ! -name '*.diff.png' ! -name '*.old.png' ! -name '*.new.png' \
    | sed 's|.*/||; s|\.png$||' \
    | sort -u)

dangling=$(comm -13 <(echo "$expected") <(echo "$on_disk"))

if [ -z "$dangling" ]; then
    echo "No dangling snapshots."
else
    echo "Dangling snapshots (on disk but not referenced in tests):"
    echo "$dangling" | while read -r name; do
        find . -path "*/tests/snapshots/${name}.png"
    done
    exit 1
fi
