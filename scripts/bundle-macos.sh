#!/usr/bin/env bash
# Build a universal (Apple Silicon + Intel) "Smart TV Toolkit.app" and a .dmg.
# Run on macOS from the repo root:  scripts/bundle-macos.sh
# Needs: rustup targets aarch64-apple-darwin and x86_64-apple-darwin.
# Optional: THIRD-PARTY-LICENSES.html in the repo root is copied into the dmg.
set -euo pipefail

APP_NAME="Smart TV Toolkit"
BIN="smart-tv-toolkit"
BUNDLE_ID="io.github.some1sm.smarttvtoolkit"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)
OUT="dist"

echo "Building $APP_NAME $VERSION (universal)…"
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

rm -rf "$OUT" && mkdir -p "$OUT"
APP="$OUT/$APP_NAME.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

lipo -create -output "$APP/Contents/MacOS/$BIN" \
    "target/aarch64-apple-darwin/release/$BIN" \
    "target/x86_64-apple-darwin/release/$BIN"
cp assets/icon.icns "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$APP_NAME</string>
    <key>CFBundleDisplayName</key><string>$APP_NAME</string>
    <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleExecutable</key><string>$BIN</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <!-- Shown when macOS asks to allow finding TVs on the local network. -->
    <key>NSLocalNetworkUsageDescription</key>
    <string>Smart TV Toolkit finds and connects to TVs on your local network.</string>
    <key>NSBonjourServices</key>
    <array>
        <string>_androidtvremote2._tcp</string>
        <string>_googlecast._tcp</string>
        <string>_adb._tcp</string>
        <string>_adb-tls-connect._tcp</string>
    </array>
</dict>
</plist>
PLIST

# Apple Silicon refuses unsigned code; an ad-hoc signature is enough to run
# (the app is not notarized, so first launch needs right-click → Open).
codesign --force --deep --sign - "$APP"

# Disk image: the app, a link to /Applications, and the license files.
STAGE="$OUT/dmg"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
cp LICENSE NOTICE.md "$STAGE/"
[ -f THIRD-PARTY-LICENSES.html ] && cp THIRD-PARTY-LICENSES.html "$STAGE/"
DMG="$OUT/SmartTVToolkit-$VERSION-macos-universal.dmg"
hdiutil create -volname "$APP_NAME" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
rm -rf "$STAGE"

echo "Done: $DMG"
