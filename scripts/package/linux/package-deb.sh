#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:-x86_64-unknown-linux-gnu}"
TAG="${2:?A release tag is required}"
[ "${TARGET}" = x86_64-unknown-linux-gnu ] || { echo 'Only Linux x86_64 packaging is supported' >&2; exit 1; }
REPO_ROOT="$(python3 -c 'from pathlib import Path; import sys; print(Path(sys.argv[1]).resolve().parents[3])' "${BASH_SOURCE[0]}")"
VERSION="$(python3 "${REPO_ROOT}/scripts/package/release.py" version "${TAG}" --field deb)"
BIN_SOURCE="${SUPERFILE_PACKAGE_BINARY:-${REPO_ROOT}/target/${TARGET}/release/superfile}"
DIST_DIR="${SUPERFILE_DIST_DIR:-${REPO_ROOT}/package-dist}"
[ -f "${BIN_SOURCE}" ] || { echo "Missing release binary: ${BIN_SOURCE}" >&2; exit 1; }
command -v dpkg-deb >/dev/null
command -v dpkg-shlibdeps >/dev/null
mkdir -p "${DIST_DIR}" "${REPO_ROOT}/target/package-staging"
STAGING_DIR="$(mktemp -d "${REPO_ROOT}/target/package-staging/linux.XXXXXX")"
trap 'python3 -c "import shutil,sys; shutil.rmtree(sys.argv[1])" "${STAGING_DIR}"' EXIT
DEB_STAGE="${STAGING_DIR}/deb"
mkdir -p "${DEB_STAGE}/DEBIAN" "${DEB_STAGE}/usr/bin" "${DEB_STAGE}/usr/share/applications" \
  "${DEB_STAGE}/usr/share/icons/hicolor/256x256/apps" "${DEB_STAGE}/usr/share/doc/superfile"
cp "${BIN_SOURCE}" "${DEB_STAGE}/usr/bin/superfile"
chmod 755 "${DEB_STAGE}/usr/bin/superfile"
cp "${REPO_ROOT}/assets/branding/superfile.png" "${DEB_STAGE}/usr/share/icons/hicolor/256x256/apps/superfile.png"
cp "${REPO_ROOT}/assets/linux/superfile.desktop" "${DEB_STAGE}/usr/share/applications/"
cp "${REPO_ROOT}/LICENSE" "${DEB_STAGE}/usr/share/doc/superfile/copyright"
cp "${REPO_ROOT}/README.md" "${DEB_STAGE}/usr/share/doc/superfile/"
# Derive the actual shared-library requirements from this runner's binary.
# subprocess.cwd avoids changing the caller's working directory.
DEPENDS="$(python3 - "${STAGING_DIR}" "${DEB_STAGE}/usr/bin/superfile" <<'PY'
from pathlib import Path
import subprocess
import sys
root = Path(sys.argv[1])
(root / 'debian').mkdir()
(root / 'debian/control').write_text('Source: superfile\n\nPackage: superfile\nArchitecture: any\nDescription: Desktop file viewer\n')
result = subprocess.run(['dpkg-shlibdeps', '-O', '-e' + sys.argv[2]], cwd=root, check=True, text=True, stdout=subprocess.PIPE)
line = next(line for line in result.stdout.splitlines() if line.startswith('shlibs:Depends='))
print(line.removeprefix('shlibs:Depends='))
PY
)"
cat > "${DEB_STAGE}/DEBIAN/control" <<EOF
Package: superfile
Version: ${VERSION}
Section: utils
Priority: optional
Architecture: amd64
Maintainer: Superfile maintainers
Depends: ${DEPENDS}, libglib2.0-0 | libglib2.0-0t64
Recommends: xdg-desktop-portal
Description: Desktop file viewer and editor
 A cross-platform file viewer and editor built with Rust and gpui-kit.
EOF
chmod -R go-w "${DEB_STAGE}"
dpkg-deb --build --root-owner-group "${DEB_STAGE}" "${DIST_DIR}/superfile-${TAG}-linux-x86_64.deb"
dpkg-deb --info "${DIST_DIR}/superfile-${TAG}-linux-x86_64.deb"

ARCHIVE_NAME="superfile-${TAG}-linux-x86_64"
TAR_STAGE="${STAGING_DIR}/${ARCHIVE_NAME}"
mkdir -p "${TAR_STAGE}"
cp "${DEB_STAGE}/usr/bin/superfile" "${TAR_STAGE}/"
cp "${REPO_ROOT}/assets/linux/superfile.desktop" "${REPO_ROOT}/assets/branding/superfile.png" \
  "${REPO_ROOT}/README.md" "${REPO_ROOT}/LICENSE" "${TAR_STAGE}/"
tar -C "${STAGING_DIR}" -czf "${DIST_DIR}/${ARCHIVE_NAME}.tar.gz" "${ARCHIVE_NAME}"
python3 "${REPO_ROOT}/scripts/package/release.py" verify "${TAG}" --platform linux-x86_64 --directory "${DIST_DIR}"
