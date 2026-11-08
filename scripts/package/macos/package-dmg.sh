#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:-aarch64-apple-darwin}"
ARCH_LABEL="${2:-aarch64}"
TAG="${3:-v0.0.1}"
CLEAN_VERSION="${TAG#v}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"

DIST_DIR="${REPO_ROOT}/package-dist"
STAGING_DIR="${REPO_ROOT}/staging"
APP_DIR="${STAGING_DIR}/Superfile.app"
BIN_SOURCE="${REPO_ROOT}/target/${TARGET}/release/superfile"

if [ ! -f "${BIN_SOURCE}" ]; then
  if [ -f "${REPO_ROOT}/target/release/superfile" ]; then
    BIN_SOURCE="${REPO_ROOT}/target/release/superfile"
  elif [ -f "${REPO_ROOT}/target/${TARGET}/debug/superfile" ]; then
    BIN_SOURCE="${REPO_ROOT}/target/${TARGET}/debug/superfile"
  elif [ -f "${REPO_ROOT}/target/debug/superfile" ]; then
    BIN_SOURCE="${REPO_ROOT}/target/debug/superfile"
  else
    echo "Error: Cannot find binary at ${BIN_SOURCE}" >&2
    exit 1
  fi
fi

mkdir -p "${DIST_DIR}"
rm -rf "${APP_DIR}"
mkdir -p "${APP_DIR}/Contents/MacOS"
mkdir -p "${APP_DIR}/Contents/Resources"

cp "${BIN_SOURCE}" "${APP_DIR}/Contents/MacOS/superfile"
chmod +x "${APP_DIR}/Contents/MacOS/superfile"

cp "${REPO_ROOT}/assets/macos/Info.plist" "${APP_DIR}/Contents/Info.plist"
cp "${REPO_ROOT}/assets/macos/Superfile.icns" "${APP_DIR}/Contents/Resources/Superfile.icns"

if command -v plutil >/dev/null 2>&1; then
  plutil -replace CFBundleShortVersionString -string "${CLEAN_VERSION}" "${APP_DIR}/Contents/Info.plist"
  plutil -replace CFBundleVersion -string "${CLEAN_VERSION}" "${APP_DIR}/Contents/Info.plist"
fi

# 1. Create .dmg disk image with Applications shortcut
DMG_STAGE="${REPO_ROOT}/dmg-stage"
rm -rf "${DMG_STAGE}"
mkdir -p "${DMG_STAGE}"
cp -R "${APP_DIR}" "${DMG_STAGE}/"
ln -s /Applications "${DMG_STAGE}/Applications"

DMG_OUTPUT="${DIST_DIR}/superfile-${TAG}-macos-${ARCH_LABEL}.dmg"
rm -f "${DMG_OUTPUT}"

if command -v hdiutil >/dev/null 2>&1; then
  echo "Creating DMG at ${DMG_OUTPUT}..."
  hdiutil create -volname "Superfile" -srcfolder "${DMG_STAGE}" -ov -format UDZO "${DMG_OUTPUT}"
else
  echo "Warning: hdiutil not available, skipping DMG creation."
fi
rm -rf "${DMG_STAGE}"

# 2. Create .tar.gz bundle
TAR_OUTPUT="${DIST_DIR}/superfile-${TAG}-macos-${ARCH_LABEL}.tar.gz"
echo "Creating tarball at ${TAR_OUTPUT}..."
tar -C "${STAGING_DIR}" -czf "${TAR_OUTPUT}" Superfile.app

echo "macOS packaging completed for ${ARCH_LABEL}."
ls -lh "${DIST_DIR}"
