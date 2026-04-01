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

# Copy universal binaries. Rename console -> tunnels for the main executable
# so that macOS displays "Tunnels" in the menu bar and About dialog.
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
</dict>
</plist>
PLIST

echo "Tunnels.app assembled at $APP (version $VERSION)"

# Convert background SVG to PNG for the DMG.
BG_PNG="$PROJECT_DIR/dist/dmg-background.png"
rsvg-convert -w 600 -h 400 "$PROJECT_DIR/resources/dmg-background.svg" > "$BG_PNG"

# Create DMG with background, icon layout, and Applications shortcut.
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

echo "DMG created at $DMG"
