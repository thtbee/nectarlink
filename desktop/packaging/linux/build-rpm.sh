#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Builds a Fedora/openSUSE .rpm package for NectarLink Desktop from the .deb
# that build-deb.sh made, so both install exactly the same files:
#   dist/nectarlink-<version>-1.<arch>.rpm
# Needs rpmbuild (the `rpm` package on Debian and Ubuntu) and dpkg-deb.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"

VERSION="${VERSION:-$(awk -F'"' '/^version[[:space:]]*=[[:space:]]*"/ { print $2; exit }' "${REPO_ROOT}/Cargo.toml")}"
if [[ -z "${VERSION}" ]]; then
    echo "error: could not determine version from ${REPO_ROOT}/Cargo.toml" >&2
    exit 1
fi

ARCH="${ARCH:-$(uname -m)}"
case "${ARCH}" in
    x86_64|amd64) RPM_ARCH="x86_64"; DEB_ARCH="amd64"; LIB_SUFFIX="()(64bit)" ;;
    aarch64|arm64) RPM_ARCH="aarch64"; DEB_ARCH="arm64"; LIB_SUFFIX="()(64bit)" ;;
    *) echo "error: unsupported architecture ${ARCH}" >&2; exit 1 ;;
esac

OUT_DIR="${OUT_DIR:-${REPO_ROOT}/dist}"
DEB="${DEB:-${OUT_DIR}/nectarlink_${VERSION}_${DEB_ARCH}.deb}"
if [[ ! -f "${DEB}" ]]; then
    echo "error: ${DEB} not found; run build-deb.sh first" >&2
    exit 1
fi

STAGE_DIR="$(mktemp -d -t nectarlink-rpm-XXXXXX)"
trap 'rm -rf "${STAGE_DIR}"' EXIT
TREE="${STAGE_DIR}/tree"
mkdir -p "${TREE}" "${STAGE_DIR}/rpmbuild"/{BUILD,RPMS,SOURCES,SPECS,SRPMS,BUILDROOT}
dpkg-deb -x "${DEB}" "${TREE}"

# What the program and its bundled libraries need from the system, as
# shared-library names any RPM distribution resolves, minus the ones that
# ship inside the package. Plus the newest glibc symbol version used.
mapfile -t ELF_FILES < <(find "${TREE}" -type f -exec sh -c 'head -c 4 "$1" | grep -q "ELF"' sh {} \; -print)
declare -A BUNDLED=()
for f in "${ELF_FILES[@]}"; do
    BUNDLED["$(basename "${f}")"]=1
    soname="$(readelf -d "${f}" 2>/dev/null | sed -n 's/.*(SONAME).*\[\(.*\)\]/\1/p')"
    [[ -n "${soname}" ]] && BUNDLED["${soname}"]=1
done
declare -A NEEDED=()
for f in "${ELF_FILES[@]}"; do
    while read -r lib; do
        [[ -n "${lib}" && -z "${BUNDLED[${lib}]:-}" ]] && NEEDED["${lib}"]=1
    done < <(readelf -d "${f}" 2>/dev/null | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p')
done
GLIBC="$(objdump -T "${TREE}/usr/lib/nectarlink/nectarlink-desktop" | grep -o 'GLIBC_[0-9.]*' | sort -V | tail -n 1)"
REQUIRES=""
for lib in $(printf '%s\n' "${!NEEDED[@]}" | sort); do
    # The loader is required through libc itself.
    [[ "${lib}" == ld-linux* ]] && continue
    REQUIRES+="Requires: ${lib}${LIB_SUFFIX}"$'\n'
done
if [[ -n "${GLIBC}" ]]; then
    REQUIRES+="Requires: libc.so.6(${GLIBC})(64bit)"$'\n'
fi

cat > "${STAGE_DIR}/rpmbuild/SPECS/nectarlink.spec" <<EOF
Name:           nectarlink
Version:        ${VERSION}
Release:        1
Summary:        Connect your Android phone and Linux PC over LAN
License:        GPL-3.0-or-later
URL:            https://github.com/alejandrokundrah/nectarlink
BuildArch:      ${RPM_ARCH}
AutoReqProv:    no
${REQUIRES}
Recommends:     polkit
Recommends:     xdg-utils
Recommends:     wl-clipboard
Recommends:     xclip
Recommends:     pipewire-utils
Recommends:     gstreamer1-plugins-base
Recommends:     avahi
Recommends:     bluez
Recommends:     fuse3

# Bundled Qt libraries are shipped as built: no stripping, debug packages
# or build-id links.
%global debug_package %{nil}
%global __os_install_post %{nil}
%global _build_id_links none

%description
NectarLink links your Android phone and Linux desktop over your local
network with end-to-end encryption: clipboard sync, notification
mirroring, file transfer, screen, camera and audio streaming, and texts
and calls.

%install
cp -a "${TREE}/usr" "%{buildroot}/"

%files
/usr/bin/nectarlink-desktop
/usr/lib/nectarlink
/usr/share/nectarlink
/usr/share/applications/nectarlink.desktop
/usr/share/metainfo/org.nectarlink.desktop.metainfo.xml
/usr/share/icons/hicolor/*/apps/nectarlink.*

%post
update-desktop-database -q /usr/share/applications >/dev/null 2>&1 || :
gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor >/dev/null 2>&1 || :

%postun
update-desktop-database -q /usr/share/applications >/dev/null 2>&1 || :
gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor >/dev/null 2>&1 || :
EOF

# Its own database: a Debian or Ubuntu host has no system one.
rpmbuild -bb --quiet \
    --define "_topdir ${STAGE_DIR}/rpmbuild" \
    --define "_dbpath ${STAGE_DIR}/rpmdb" \
    --define "_rpmfilename %%{NAME}-%%{VERSION}-%%{RELEASE}.%%{ARCH}.rpm" \
    "${STAGE_DIR}/rpmbuild/SPECS/nectarlink.spec"

mkdir -p "${OUT_DIR}"
OUT_RPM="${OUT_DIR}/nectarlink-${VERSION}-1.${RPM_ARCH}.rpm"
cp "${STAGE_DIR}/rpmbuild/RPMS/nectarlink-${VERSION}-1.${RPM_ARCH}.rpm" "${OUT_RPM}"
echo "Built ${OUT_RPM}"
