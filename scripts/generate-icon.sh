#!/bin/bash
# Generate Tunnels.icns from the SVG source.
# Requires: brew install librsvg
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
SVG="$PROJECT_DIR/resources/tunnels-icon.svg"
ICNS="$PROJECT_DIR/resources/Tunnels.icns"
ICONSET=$(mktemp -d)/Tunnels.iconset
mkdir -p "$ICONSET"

for size in 16 32 128 256 512; do
  rsvg-convert -w "$size" -h "$size" "$SVG" > "$ICONSET/icon_${size}x${size}.png"
  double=$((size * 2))
  rsvg-convert -w "$double" -h "$double" "$SVG" > "$ICONSET/icon_${size}x${size}@2x.png"
done

iconutil -c icns "$ICONSET" -o "$ICNS"
rm -rf "$(dirname "$ICONSET")"

echo "Generated $ICNS"
