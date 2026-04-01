#!/bin/bash
# Assemble Tunnels.app from universal binaries in dist/ and create a DMG.
# Requires VERSION env var (e.g. VERSION=2026.04.01-1).
set -e

VERSION="${VERSION:?VERSION env var is required (e.g. 2026.04.01-1)}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
APP="$PROJECT_DIR/dist/Tunnels.app"

# Clean previous bundle.
rm -rf "$APP"

# Create bundle structure.
mkdir -p "$APP/Contents/MacOS"
mkdir -p "$APP/Contents/Resources"

# Copy universal binaries.
for bin in console tunnelclient tunnel-bootstrap bootstrap-deploy; do
  cp "$PROJECT_DIR/dist/$bin" "$APP/Contents/MacOS/$bin"
  chmod +x "$APP/Contents/MacOS/$bin"
done

# Copy icon.
cp "$PROJECT_DIR/resources/Tunnels.icns" "$APP/Contents/Resources/Tunnels.icns"

# Generate Info.plist.
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>console</string>
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
</dict>
</plist>
PLIST

echo "Tunnels.app assembled at $APP (version $VERSION)"

# Create DMG.
DMG="$PROJECT_DIR/dist/Tunnels.dmg"
rm -f "$DMG"
hdiutil create -volname "Tunnels" -srcfolder "$APP" -ov -format UDZO "$DMG"

echo "DMG created at $DMG"
