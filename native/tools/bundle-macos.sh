#!/bin/sh
# Builds a universal (Apple Silicon + Intel) "Wallpaper Switcher.app" and a .dmg to share it.
# Usage: native/tools/bundle-macos.sh
# Writes native/target/release/Wallpaper Switcher.app and Wallpaper-Switcher-<version>-mac.dmg
# Needs: rustup target add aarch64-apple-darwin x86_64-apple-darwin
set -eu

NATIVE="$(cd "$(dirname "$0")/.." && pwd)"
cd "$NATIVE"

cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
APP="target/release/Wallpaper Switcher.app"
DMG="target/release/Wallpaper-Switcher-$VERSION-mac.dmg"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/wallswitch" \
    target/aarch64-apple-darwin/release/wallswitch \
    target/x86_64-apple-darwin/release/wallswitch

# icon: the largest PNG inside icon.ico (256px) scaled into an iconset
python3 -I - assets/icon.ico "$WORK/icon.png" <<'EOF'
import struct, sys
data = open(sys.argv[1], 'rb').read()
count = struct.unpack_from('<H', data, 4)[0]
best = (0, b'')
for i in range(count):
    w, _, _, _, _, _, size, off = struct.unpack_from('<BBBBHHII', data, 6 + 16 * i)
    w = w or 256
    if w > best[0]:
        best = (w, data[off:off + size])
open(sys.argv[2], 'wb').write(best[1])
EOF
ICONSET="$WORK/AppIcon.iconset"
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
    sips -z $size $size "$WORK/icon.png" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z $double $double "$WORK/icon.png" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>Wallpaper Switcher</string>
  <key>CFBundleDisplayName</key>
  <string>Wallpaper Switcher</string>
  <key>CFBundleIdentifier</key>
  <string>com.wallpaperswitcher.app</string>
  <key>CFBundleExecutable</key>
  <string>wallswitch</string>
  <key>CFBundleIconFile</key>
  <string>AppIcon</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>$VERSION</string>
  <key>CFBundleVersion</key>
  <string>$VERSION</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>LSMinimumSystemVersion</key>
  <string>11.0</string>
  <key>LSUIElement</key>
  <true/>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
EOF

# ad-hoc signature so Apple Silicon will launch it; not notarized
codesign --force --deep --sign - "$APP"

# disk image with the app next to an Applications shortcut
STAGE="$WORK/dmg"
mkdir -p "$STAGE"
ditto "$APP" "$STAGE/Wallpaper Switcher.app"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
hdiutil create -quiet -volname "Wallpaper Switcher" -srcfolder "$STAGE" -fs HFS+ -format UDZO "$DMG"

echo "Built $NATIVE/$APP"
echo "Built $NATIVE/$DMG"
