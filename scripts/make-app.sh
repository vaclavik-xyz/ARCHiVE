#!/usr/bin/env bash
# Build ARCHiVE.app — a double-clickable macOS wrapper around `archive ui`.
#
#   scripts/make-app.sh [--version N.N.N] [--skip-notarize]
#
# - builds `archive` + `imessage-exporter` universal (arm64 + x86_64 via lipo)
# - compiles the Swift launcher universal
# - assembles ARCHiVE.app with the brand icns
# - signs with Developer ID (hardened runtime) when the identity is available
# - notarizes + staples when a notarytool keychain profile named
#   ARCHIVE_NOTARY exists (one-time setup:
#   xcrun notarytool store-credentials ARCHIVE_NOTARY ...)
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"
VER="$(grep -m1 '^version' archive/Cargo.toml | cut -d'"' -f2)"
BUILD="$ROOT/build"
APP="$BUILD/ARCHiVE.app"
IDENTITY_NAME="Developer ID Application: Filip Václavík (96CM57BHVD)"
NOTARY_PROFILE="ARCHIVE_NOTARY"
SKIP_NOTARIZE=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) VER="$2"; shift 2 ;;
    --skip-notarize) SKIP_NOTARIZE=1; shift ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

echo "▸ ARCHiVE.app v$VER"

# 1. Universal CLI binaries (build both architectures, lipo together).
echo "▸ building archive (universal)…"
cargo build --release --target aarch64-apple-darwin --quiet
cargo build --release --target x86_64-apple-darwin --quiet
mkdir -p "$BUILD/universal"
for BIN in archive imessage-exporter; do
  lipo -create \
    "target/aarch64-apple-darwin/release/$BIN" \
    "target/x86_64-apple-darwin/release/$BIN" \
    -output "build/universal/$BIN"
done

# 2. Universal launcher.
echo "▸ compiling launcher…"
swiftc -O -target arm64-apple-macos13 -o build/universal/ARCHiVE-launcher-arm64 scripts/app-launcher.swift
swiftc -O -target x86_64-apple-macos13 -o build/universal/ARCHiVE-launcher-x64 scripts/app-launcher.swift
lipo -create build/universal/ARCHiVE-launcher-arm64 build/universal/ARCHiVE-launcher-x64 \
  -output build/universal/ARCHiVE-launcher

# 3. Icon: brand PNG → icns (16..1024 px).
echo "▸ making icon…"
ICONSET="$BUILD/icon.iconset"
rm -rf "$ICONSET"; mkdir -p "$ICONSET"
for S in 16 32 64 128 256 512; do
  sips -z "$S" "$S" docs/brand/favicon.png --out "$ICONSET/icon_${S}x${S}.png" >/dev/null
  D=$((S * 2))
  sips -z "$D" "$D" docs/brand/favicon.png --out "$ICONSET/icon_${S}x${S}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$BUILD/icon.icns"

# 4. Assemble the bundle.
echo "▸ assembling ARCHiVE.app…"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp build/universal/ARCHiVE-launcher "$APP/Contents/MacOS/ARCHiVE"
cp build/universal/archive build/universal/imessage-exporter "$APP/Contents/Resources/"
cp "$BUILD/icon.icns" "$APP/Contents/Resources/"
chmod +x "$APP/Contents/Resources/archive" "$APP/Contents/Resources/imessage-exporter"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>                <string>ARCHiVE</string>
  <key>CFBundleDisplayName</key>         <string>ARCHiVE</string>
  <key>CFBundleIdentifier</key>          <string>xyz.vaclavik.archive</string>
  <key>CFBundleVersion</key>             <string>$VER</string>
  <key>CFBundleShortVersionString</key>  <string>$VER</string>
  <key>CFBundleExecutable</key>          <string>ARCHiVE</string>
  <key>CFBundleIconFile</key>            <string>icon.icns</string>
  <key>CFBundlePackageType</key>         <string>APPL</string>
  <key>LSMinimumSystemVersion</key>      <string>13.0</string>
  <key>NSHighResolutionCapable</key>     <true/>
  <key>LSApplicationCategoryType</key>   <string>public.app-category.utilities</string>
  <!-- Drag-and-drop of a backup folder onto the icon prefills --backup. -->
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>LSItemContentTypes</key>      <array><string>public.folder</string></array>
      <key>CFBundleTypeName</key>        <string>iOS záloha</string>
      <key>CFBundleTypeRole</key>        <string>Viewer</string>
    </dict>
  </array>
</dict>
</plist>
PLIST

# 5. Sign (Developer ID when available, adhoc otherwise).
if security find-identity -v -p codesigning | grep -q "$IDENTITY_NAME"; then
  echo "▸ signing (Developer ID)…"
  # Nested helpers first, then the outer bundle.
  codesign --force --options runtime --timestamp --sign "$IDENTITY_NAME" \
    "$APP/Contents/Resources/archive" "$APP/Contents/Resources/imessage-exporter"
  codesign --force --options runtime --timestamp --sign "$IDENTITY_NAME" "$APP"
  SIGNED=1
else
  echo "▸ Developer ID not found — adhoc signing only (no notarization)."
  codesign --force --deep --sign - "$APP"
  SIGNED=0
fi

# 6. Notarize + staple.
#    Credentials, in order of preference:
#    a) App Store Connect API key: env vars ARCHIVE_ASC_KEY (path to .p8),
#       ARCHIVE_ASC_KEY_ID, ARCHIVE_ASC_ISSUER
#    b) a notarytool keychain profile named $NOTARY_PROFILE (one-time setup:
#       xcrun notarytool store-credentials ARCHIVE_NOTARY ...)
if [[ $SIGNED == 1 && $SKIP_NOTARIZE == 0 ]]; then
  echo "▸ notarizing…"
  Z="$BUILD/ARCHiVE-app.zip"
  ditto -c -k --keepParent "$APP" "$Z"
  NOTARY_ARGS=()
  if [[ -n "${ARCHIVE_ASC_KEY:-}" && -n "${ARCHIVE_ASC_KEY_ID:-}" && -n "${ARCHIVE_ASC_ISSUER:-}" ]]; then
    NOTARY_ARGS=(--key "$ARCHIVE_ASC_KEY" --key-id "$ARCHIVE_ASC_KEY_ID" --issuer "$ARCHIVE_ASC_ISSUER")
  elif [[ -n "${ARCHIVE_ASC_KEYCHAIN_PROFILE:-}" ]]; then
    NOTARY_ARGS=(--keychain-profile "$ARCHIVE_ASC_KEYCHAIN_PROFILE")
  else
    NOTARY_ARGS=(--keychain-profile "$NOTARY_PROFILE")
  fi
  if xcrun notarytool submit "$Z" "${NOTARY_ARGS[@]}" --wait; then
    # A transient stapler failure must not sink the build: the notarization
    # ticket online is enough for Gatekeeper, the staple is a cache.
    if xcrun stapler staple "$APP"; then
      echo "▸ stapled ✓"
    else
      echo "!! stapling failed (transient?) — app is still notarized" >&2
    fi
  else
    # A signed-but-unnotarized app gets hard-blocked by Gatekeeper — refuse to
    # produce a distribution zip from a failed notarization run.
    echo "!! notarization failed — NOT building the distribution zip" >&2
    exit 3
  fi
fi

# 7. Distribution zip (ditto keeps the quarantine-xattr-safe layout).
ZIP="$ROOT/build/ARCHiVE-macos-universal-v$VER.zip"
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
echo "▸ $ZIP"
