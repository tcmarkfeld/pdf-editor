#!/bin/sh
# Builds dist/Reflow.app. Options:
#   --dmg      also build dist/Reflow.dmg
#   --install  copy the app to /Applications and register it with Launch
#              Services (so Finder's Open With offers it for PDFs)
# The bundle is self-contained: PDFium ships in Contents/Frameworks, where the
# app looks for it first. Signed ad-hoc, which is enough to run on this Mac;
# distributing to other Macs needs a Developer ID signature + notarization.
set -eu
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

[ -f vendor/pdfium/lib/libpdfium.dylib ] || scripts/fetch-pdfium.sh
cargo build --release -p app

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
app=dist/Reflow.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Frameworks" "$app/Contents/Resources"
cp target/release/reflow "$app/Contents/MacOS/reflow"
cp vendor/pdfium/lib/libpdfium.dylib "$app/Contents/Frameworks/"

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Reflow</string>
  <key>CFBundleDisplayName</key><string>Reflow</string>
  <key>CFBundleIdentifier</key><string>com.tcmarkfeld.reflow</string>
  <key>CFBundleExecutable</key><string>reflow</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleIconFile</key><string>Reflow</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>PDF Document</string>
      <key>CFBundleTypeRole</key><string>Editor</string>
      <key>LSItemContentTypes</key><array><string>com.adobe.pdf</string></array>
      <!-- Alternate: offered in Open With without replacing the default viewer. -->
      <key>LSHandlerRank</key><string>Alternate</string>
    </dict>
    <dict>
      <key>CFBundleTypeName</key><string>Reflow Document</string>
      <key>CFBundleTypeRole</key><string>Editor</string>
      <key>LSItemContentTypes</key><array><string>com.tcmarkfeld.reflow.document</string></array>
      <key>LSHandlerRank</key><string>Owner</string>
    </dict>
  </array>
  <key>UTExportedTypeDeclarations</key>
  <array>
    <dict>
      <key>UTTypeIdentifier</key><string>com.tcmarkfeld.reflow.document</string>
      <key>UTTypeDescription</key><string>Reflow Document</string>
      <key>UTTypeConformsTo</key><array><string>public.data</string></array>
      <key>UTTypeTagSpecification</key>
      <dict><key>public.filename-extension</key><array><string>reflow</string></array></dict>
    </dict>
  </array>
</dict>
</plist>
EOF

# Icon: a simple generated page glyph (replace scripts/icon.png to customise).
if [ -f scripts/icon.png ]; then
  iconset=$(mktemp -d)/Reflow.iconset
  mkdir -p "$iconset"
  for s in 16 32 128 256 512; do
    sips -z $s $s scripts/icon.png --out "$iconset/icon_${s}x${s}.png" >/dev/null
    sips -z $((s * 2)) $((s * 2)) scripts/icon.png --out "$iconset/icon_${s}x${s}@2x.png" >/dev/null
  done
  iconutil -c icns "$iconset" -o "$app/Contents/Resources/Reflow.icns"
fi

codesign --force --sign - "$app/Contents/Frameworks/libpdfium.dylib"
codesign --force --sign - "$app"
echo "Built $app"

lsregister=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
# Keep only the installed copy registered, so Open With launches it.
"$lsregister" -u "$app" >/dev/null 2>&1 || true

for arg in "$@"; do
  if [ "$arg" = "--install" ]; then
    rm -rf /Applications/Reflow.app
    cp -R "$app" /Applications/
    "$lsregister" -f /Applications/Reflow.app
    echo "Installed /Applications/Reflow.app"
  fi
done

if echo " $* " | grep -q -- " --dmg "; then
  rm -f dist/Reflow.dmg
  staging=$(mktemp -d)
  cp -R "$app" "$staging/"
  ln -s /Applications "$staging/Applications"
  hdiutil create -volname Reflow -srcfolder "$staging" -ov -format UDZO dist/Reflow.dmg >/dev/null
  echo "Built dist/Reflow.dmg"
fi
