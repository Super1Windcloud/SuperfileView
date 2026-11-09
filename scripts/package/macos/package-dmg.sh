#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:-aarch64-apple-darwin}"
ARCH_LABEL="${2:-aarch64}"
TAG="${3:-v0.0.2}"
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
  echo "DMG staging $(du -sh "${DMG_STAGE}" | awk '{print $1}'), host free $(df -h / | awk 'NR==2 {print $4}')"

  # macOS 11+ sizes the default APFS `-srcfolder` image too tightly. Copying
  # the app then fails on GitHub-hosted runners with:
  #   hdiutil: create failed - No space left on device
  #   could not access /Volumes/Superfile/.../superfile - No space left on device
  # even when the host volume still has room. HFS+ keeps the layout hdiutil
  # estimates correctly (actions/runner-images#4288). Retry because those
  # runners also race Spotlight/XProtect and return "Resource busy".
  attempt=1
  max_attempts=3
  while true; do
    if hdiutil create \
      -volname "Superfile" \
      -srcfolder "${DMG_STAGE}" \
      -ov \
      -fs HFS+ \
      -format UDZO \
      "${DMG_OUTPUT}"; then
      break
    fi
    if [ "${attempt}" -ge "${max_attempts}" ]; then
      echo "Error: hdiutil create failed after ${max_attempts} attempts." >&2
      df -h / >&2 || true
      exit 1
    fi
    echo "hdiutil create failed (attempt ${attempt}/${max_attempts}); retrying..." >&2
    hdiutil detach "/Volumes/Superfile" >/dev/null 2>&1 || true
    rm -f "${DMG_OUTPUT}"
    attempt=$((attempt + 1))
    sleep $((attempt * 2))
  done
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
