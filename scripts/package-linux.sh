#!/usr/bin/env bash
# Build the Linux (x86_64) release as a .tar.gz with a desktop entry.
# Run from the repo root:  scripts/package-linux.sh
# Optional: THIRD-PARTY-LICENSES.html in the repo root is included.
set -euo pipefail

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)
NAME="SmartTVToolkit-$VERSION-linux-x86_64"

echo "Building Smart TV Toolkit $VERSION for Linux…"
cargo build --release --locked

rm -rf dist && mkdir -p "dist/$NAME"
STAGE="dist/$NAME"
cp target/release/smart-tv-toolkit "$STAGE/"
cp assets/icon-256.png "$STAGE/smart-tv-toolkit.png"
cp LICENSE NOTICE.md "$STAGE/"
[ -f THIRD-PARTY-LICENSES.html ] && cp THIRD-PARTY-LICENSES.html "$STAGE/"

cat > "$STAGE/smart-tv-toolkit.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Smart TV Toolkit
Comment=Manage smart TVs from your computer
Exec=smart-tv-toolkit
Icon=smart-tv-toolkit
Categories=Utility;
Terminal=false
DESKTOP

cat > "$STAGE/install.sh" <<'INSTALL'
#!/usr/bin/env sh
# Install for the current user: binary, icon and menu entry.
set -e
cd "$(dirname "$0")"
mkdir -p ~/.local/bin ~/.local/share/applications ~/.local/share/icons/hicolor/256x256/apps
install -m 755 smart-tv-toolkit ~/.local/bin/
install -m 644 smart-tv-toolkit.png ~/.local/share/icons/hicolor/256x256/apps/
install -m 644 smart-tv-toolkit.desktop ~/.local/share/applications/
echo "Installed. Start it from your app menu, or run: smart-tv-toolkit"
INSTALL
chmod +x "$STAGE/install.sh" "$STAGE/smart-tv-toolkit"

tar -C dist -czf "dist/$NAME.tar.gz" "$NAME"
rm -rf "$STAGE"
echo "Done: dist/$NAME.tar.gz"
