#!/bin/sh
# Downloads the prebuilt PDFium matching pdfium-render's `pdfium_7881` feature
# into vendor/pdfium (BSD-3/Apache-2.0, from bblanchon/pdfium-binaries).
set -eu
VERSION=7881
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) PLATFORM=mac-arm64 ;;
  Darwin-x86_64) PLATFORM=mac-x64 ;;
  Linux-x86_64) PLATFORM=linux-x64 ;;
  Linux-aarch64) PLATFORM=linux-arm64 ;;
  *) echo "unsupported platform; set PDFIUM_DYNAMIC_LIB_PATH manually" >&2; exit 1 ;;
esac
root="$(cd "$(dirname "$0")/.." && pwd)"
mkdir -p "$root/vendor/pdfium"
curl -sSfL "https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F$VERSION/pdfium-$PLATFORM.tgz" \
  | tar -xz -C "$root/vendor/pdfium"
echo "PDFium $VERSION installed in vendor/pdfium"
