#!/bin/bash
# Build the complete Tunnels.app bundle and DMG from scratch.
# Usage: VERSION=2026.04.01-1 scripts/build-app.sh
#
# Prerequisites: brew install cmake librsvg create-dmg
#                rustup target add x86_64-apple-darwin aarch64-apple-darwin
set -e

VERSION="${VERSION:?VERSION env var is required (e.g. 2026.04.01-1)}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# --- Generate icon ---

echo "==> Generating icon SVG..."
python3 "$SCRIPT_DIR/generate-icon-svg.py"

echo "==> Converting SVG to icns..."
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

# --- Build universal binaries ---

echo "==> Building universal binaries..."
export MACOSX_DEPLOYMENT_TARGET=10.13

PACKAGES="-p console -p tunnelclient -p tunnel-bootstrap -p bootstrap-deploy"
cargo build --release --target x86_64-apple-darwin $PACKAGES
cargo build --release --target aarch64-apple-darwin $PACKAGES

mkdir -p "$PROJECT_DIR/dist"

for bin in console tunnelclient tunnel-bootstrap bootstrap-deploy; do
  lipo -create \
    "$PROJECT_DIR/target/x86_64-apple-darwin/release/$bin" \
    "$PROJECT_DIR/target/aarch64-apple-darwin/release/$bin" \
    -output "$PROJECT_DIR/dist/$bin"
done

# --- Assemble app bundle ---

echo "==> Assembling Tunnels.app..."
APP="$PROJECT_DIR/dist/Tunnels.app"
rm -rf "$APP"

mkdir -p "$APP/Contents/MacOS"
mkdir -p "$APP/Contents/Resources"

# Rename console -> Tunnels so macOS displays "Tunnels" in menus.
cp "$PROJECT_DIR/dist/console" "$APP/Contents/MacOS/Tunnels"
chmod +x "$APP/Contents/MacOS/Tunnels"
for bin in tunnelclient tunnel-bootstrap bootstrap-deploy; do
  cp "$PROJECT_DIR/dist/$bin" "$APP/Contents/MacOS/$bin"
  chmod +x "$APP/Contents/MacOS/$bin"
done

# Helper script for viewing logs.
cat > "$APP/Contents/MacOS/view-logs.sh" <<'LOGSCRIPT'
#!/bin/bash
log stream --predicate 'subsystem == "com.generalelectrix.tunnels"'
LOGSCRIPT
chmod +x "$APP/Contents/MacOS/view-logs.sh"

cp "$ICNS" "$APP/Contents/Resources/Tunnels.icns"
cp "$PROJECT_DIR/controller_templates/tunnels.touchosc" "$APP/Contents/Resources/tunnels.touchosc"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>Tunnels</string>
    <key>CFBundleIdentifier</key>
    <string>com.generalelectrix.tunnels</string>
    <key>CFBundleName</key>
    <string>Tunnels</string>
    <key>CFBundleDisplayName</key>
    <string>Tunnels</string>
    <key>CFBundleVersion</key>
    <string>${VERSION}</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleIconFile</key>
    <string>Tunnels</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>LSMinimumSystemVersion</key>
    <string>10.13</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSMicrophoneUsageDescription</key>
    <string>Tunnels hungers for your audio input.</string>
    <key>NSLocalNetworkUsageDescription</key>
    <string>Tunnels gotta use your network, yo.</string>
</dict>
</plist>
PLIST

echo "==> Signing app bundle..."
codesign -s - --force --deep --identifier com.generalelectrix.tunnels "$APP"

echo "==> Creating DMG..."
BG_PNG="$PROJECT_DIR/dist/dmg-background.png"
rsvg-convert -w 600 -h 400 "$PROJECT_DIR/resources/dmg-background.svg" > "$BG_PNG"

DMG="$PROJECT_DIR/dist/Tunnels.dmg"
rm -f "$DMG"
create-dmg \
  --volname "Tunnels" \
  --background "$BG_PNG" \
  --window-size 600 400 \
  --icon-size 128 \
  --icon "Tunnels.app" 150 210 \
  --app-drop-link 450 210 \
  "$DMG" "$APP"
rm -f "$BG_PNG"

echo "==> Done: $DMG"
