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
python3 "${REPO_ROOT}/scripts/package/macos/bundle.py" "${BIN_SOURCE}" "${APP_DIR}" "${VERSION}"
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
