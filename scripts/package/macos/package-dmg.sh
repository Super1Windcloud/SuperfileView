#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:-aarch64-apple-darwin}"
ARCH_LABEL="${2:-aarch64}"
TAG="${3:?A release tag is required}"
REPO_ROOT="$(python3 -c 'from pathlib import Path; import sys; print(Path(sys.argv[1]).resolve().parents[3])' "${BASH_SOURCE[0]}")"
VERSION="$(python3 "${REPO_ROOT}/scripts/package/release.py" version "${TAG}" --field core)"
case "${TARGET}:${ARCH_LABEL}" in
  aarch64-apple-darwin:aarch64) MACH_ARCH=arm64 ;;
  x86_64-apple-darwin:x86_64) MACH_ARCH=x86_64 ;;
  *) echo 'Unsupported macOS target/architecture pair' >&2; exit 1 ;;
esac

# An explicit override is useful for local packaging smoke tests. CI always uses
# the exact release target; missing builds must never fall back to debug binaries.
BIN_SOURCE="${SUPERFILE_PACKAGE_BINARY:-${REPO_ROOT}/target/${TARGET}/release/superfile}"
DIST_DIR="${SUPERFILE_DIST_DIR:-${REPO_ROOT}/package-dist}"
[ -f "${BIN_SOURCE}" ] || { echo "Missing release binary: ${BIN_SOURCE}" >&2; exit 1; }
command -v hdiutil >/dev/null
command -v codesign >/dev/null
lipo "${BIN_SOURCE}" -verify_arch "${MACH_ARCH}"
mkdir -p "${DIST_DIR}" "${REPO_ROOT}/target/package-staging"
STAGING_DIR="$(mktemp -d "${REPO_ROOT}/target/package-staging/macos.XXXXXX")"
trap 'python3 -c "import shutil,sys; shutil.rmtree(sys.argv[1])" "${STAGING_DIR}"' EXIT
APP_DIR="${STAGING_DIR}/Superfile.app"
mkdir -p "${APP_DIR}/Contents/MacOS" "${APP_DIR}/Contents/Resources"
cp "${BIN_SOURCE}" "${APP_DIR}/Contents/MacOS/superfile"
chmod 755 "${APP_DIR}/Contents/MacOS/superfile"
cp "${REPO_ROOT}/assets/macos/Superfile.icns" "${APP_DIR}/Contents/Resources/Superfile.icns"
cp "${REPO_ROOT}/LICENSE" "${REPO_ROOT}/README.md" "${APP_DIR}/Contents/Resources/"
python3 - "${REPO_ROOT}/assets/macos/Info.plist" "${APP_DIR}/Contents/Info.plist" "${VERSION}" <<'PY'
from pathlib import Path
import plistlib
import sys
with open(sys.argv[1], 'rb') as file:
    info = plistlib.load(file)
info['CFBundleShortVersionString'] = sys.argv[3]
info['CFBundleVersion'] = sys.argv[3]
Path(sys.argv[2]).write_bytes(plistlib.dumps(info, sort_keys=False))
PY
# Ad-hoc signing gives the bundle a valid local signature. Developer ID signing
# and Apple notarization require separate credentials and are not configured.
codesign --force --sign - "${APP_DIR}"
codesign --verify --deep --strict "${APP_DIR}"
ln -s /Applications "${STAGING_DIR}/Applications"

DMG_OUTPUT="${DIST_DIR}/superfile-${TAG}-macos-${ARCH_LABEL}.dmg"
# HFS+ avoids hdiutil's too-small APFS source-folder images on hosted runners.
for attempt in 1 2 3; do
  if hdiutil create -volname Superfile -srcfolder "${STAGING_DIR}" -ov -fs HFS+ -format UDZO "${DMG_OUTPUT}"; then
    break
  fi
  if [ "${attempt}" -eq 3 ]; then
    echo 'DMG creation failed after three attempts' >&2
    exit 1
  fi
  sleep 2
done
hdiutil verify "${DMG_OUTPUT}"
COPYFILE_DISABLE=1 tar -C "${STAGING_DIR}" -czf "${DIST_DIR}/superfile-${TAG}-macos-${ARCH_LABEL}.tar.gz" Superfile.app
python3 "${REPO_ROOT}/scripts/package/release.py" verify "${TAG}" --platform "macos-${ARCH_LABEL}" --directory "${DIST_DIR}"
