#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:-x86_64-unknown-linux-gnu}"
TAG="${2:-v0.1.0}"
CLEAN_VERSION="${TAG#v}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"

DIST_DIR="${REPO_ROOT}/package-dist"
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

# 1. Build .deb package
DEB_STAGE="${REPO_ROOT}/deb-stage"
rm -rf "${DEB_STAGE}"
mkdir -p "${DEB_STAGE}/DEBIAN"
mkdir -p "${DEB_STAGE}/usr/bin"
mkdir -p "${DEB_STAGE}/usr/share/applications"
mkdir -p "${DEB_STAGE}/usr/share/icons/hicolor/256x256/apps"
mkdir -p "${DEB_STAGE}/usr/share/doc/superfile"

cat << EOF > "${DEB_STAGE}/DEBIAN/control"
Package: superfile
Version: ${CLEAN_VERSION}
Section: utils
Priority: optional
Architecture: amd64
Maintainer: Superfile Team <https://github.com/superfile>
Description: High-performance desktop file viewer and editor
 Superfile is a fast, cross-platform desktop file viewer and editor
 for documents, code, images, books, audio/video, and structured data,
 built with Rust and gpui-kit.
EOF

cp "${BIN_SOURCE}" "${DEB_STAGE}/usr/bin/superfile"
chmod 755 "${DEB_STAGE}/usr/bin/superfile"

if [ -f "${REPO_ROOT}/assets/branding/superfile.png" ]; then
  cp "${REPO_ROOT}/assets/branding/superfile.png" "${DEB_STAGE}/usr/share/icons/hicolor/256x256/apps/superfile.png"
fi

if [ -f "${REPO_ROOT}/assets/linux/superfile.desktop" ]; then
  cp "${REPO_ROOT}/assets/linux/superfile.desktop" "${DEB_STAGE}/usr/share/applications/superfile.desktop"
fi

if [ -f "${REPO_ROOT}/LICENSE" ]; then
  cp "${REPO_ROOT}/LICENSE" "${DEB_STAGE}/usr/share/doc/superfile/copyright"
fi
if [ -f "${REPO_ROOT}/README.md" ]; then
  cp "${REPO_ROOT}/README.md" "${DEB_STAGE}/usr/share/doc/superfile/"
fi

DEB_OUTPUT="${DIST_DIR}/superfile-${TAG}-linux-x86_64.deb"
rm -f "${DEB_OUTPUT}"

if command -v dpkg-deb >/dev/null 2>&1; then
  echo "Building Debian package at ${DEB_OUTPUT}..."
  dpkg-deb --build --root-owner-group "${DEB_STAGE}" "${DEB_OUTPUT}"
else
  echo "Warning: dpkg-deb not found, skipping .deb creation."
fi
rm -rf "${DEB_STAGE}"

# 2. Build .tar.gz universal archive
TAR_STAGE="${REPO_ROOT}/tar-stage/superfile-${TAG}-linux-x86_64"
rm -rf "${REPO_ROOT}/tar-stage"
mkdir -p "${TAR_STAGE}"

cp "${BIN_SOURCE}" "${TAR_STAGE}/superfile"
chmod 755 "${TAR_STAGE}/superfile"

if [ -f "${REPO_ROOT}/assets/linux/superfile.desktop" ]; then
  cp "${REPO_ROOT}/assets/linux/superfile.desktop" "${TAR_STAGE}/"
fi
if [ -f "${REPO_ROOT}/assets/branding/superfile.png" ]; then
  cp "${REPO_ROOT}/assets/branding/superfile.png" "${TAR_STAGE}/"
fi
if [ -f "${REPO_ROOT}/README.md" ]; then
  cp "${REPO_ROOT}/README.md" "${TAR_STAGE}/"
fi
if [ -f "${REPO_ROOT}/LICENSE" ]; then
  cp "${REPO_ROOT}/LICENSE" "${TAR_STAGE}/"
fi

TAR_OUTPUT="${DIST_DIR}/superfile-${TAG}-linux-x86_64.tar.gz"
echo "Creating universal Linux tarball at ${TAR_OUTPUT}..."
tar -C "${REPO_ROOT}/tar-stage" -czf "${TAR_OUTPUT}" "superfile-${TAG}-linux-x86_64"
rm -rf "${REPO_ROOT}/tar-stage"

echo "Linux packaging completed."
ls -lh "${DIST_DIR}"
