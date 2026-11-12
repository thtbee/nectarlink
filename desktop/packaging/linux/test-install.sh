#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Tests install.sh and the package scripts without building the app: makes
# the .deb, .rpm and tarball around a stand-in program (/bin/true), then has
# install.sh install and remove them in clean containers, as root on Ubuntu
# 24.04, Fedora 40 and Arch, and as a user without sudo.
#
# Needs dpkg-deb, rpmbuild, readelf, Qt lrelease, Python 3 and docker
# (or podman: DOCKER=podman).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DOCKER="${DOCKER:-docker}"
WORK="$(mktemp -d -t nectarlink-test-install-XXXXXX)"
trap 'rm -rf "${WORK}"' EXIT
DIST="${WORK}/dist"
mkdir -p "${DIST}" "${WORK}/qt"/{lib,plugins,qml}

# A private Qt that's empty, so the packages need no Qt from the system.
cp /bin/true "${WORK}/nectarlink-desktop"
printf '%s\n' 'stale catalogue' > "${WORK}/nectarlink_xx.qm"
cat > "${WORK}/qmake" <<EOF
#!/bin/sh
case "\$2" in
    QT_INSTALL_LIBS) echo "${WORK}/qt/lib" ;;
    QT_INSTALL_PLUGINS) echo "${WORK}/qt/plugins" ;;
    QT_INSTALL_QML) echo "${WORK}/qt/qml" ;;
esac
EOF
chmod +x "${WORK}/qmake"

export BIN_PATH="${WORK}/nectarlink-desktop" QMAKE="${WORK}/qmake" OUT_DIR="${DIST}" VERSION=9.9.9
"${SCRIPT_DIR}/build-deb.sh" >/dev/null
"${SCRIPT_DIR}/build-rpm.sh" >/dev/null
"${SCRIPT_DIR}/build-tarball.sh" >/dev/null
# A clean build must generate and ship Brazilian Portuguese, rather than
# depend on .qm files left over from a previous release build.
dpkg-deb -x "${DIST}/nectarlink_9.9.9_$(dpkg --print-architecture).deb" "${WORK}/deb-content"
test -s "${WORK}/deb-content/usr/share/nectarlink/nectarlink_pt_BR.qm"
test ! -e "${WORK}/deb-content/usr/share/nectarlink/nectarlink_xx.qm"
tar -tzf "${DIST}/Nectarlink-9.9.9-linux-$(uname -m).tar.gz" > "${WORK}/tar-content"
grep -q '/share/nectarlink/nectarlink_pt_BR.qm$' "${WORK}/tar-content"
! grep -q '/nectarlink_xx.qm$' "${WORK}/tar-content"
cp "${SCRIPT_DIR}/install.sh" "${DIST}/install.sh"
(cd "${DIST}" && sha256sum ./*.deb ./*.rpm ./*.tar.gz | sed 's| \./| |' > SHA256SUMS.txt)
chmod -R a+rX "${WORK}"

check() {
    local name="$1" image="$2" script="$3"
    echo "--- ${name} (${image})"
    if "${DOCKER}" run --rm -v "${DIST}:/dist:ro" -e NECTARLINK_ASSETS_DIR=/dist "${image}" sh -euc "${script}" \
        > "${WORK}/${name}.log" 2>&1; then
        grep -E '^(\[|      [A-Z])' "${WORK}/${name}.log" || true
        echo "--- ${name}: passed"
    else
        cat "${WORK}/${name}.log"
        echo "--- ${name}: FAILED" >&2
        exit 1
    fi
}

ROOT_ROUND_TRIP='
    sh /dist/install.sh
    nectarlink-desktop --version >/dev/null
    test -f /etc/udev/rules.d/70-nectarlink-uinput.rules
    sh /dist/install.sh --uninstall
    ! command -v nectarlink-desktop
    ! test -e /etc/udev/rules.d/70-nectarlink-uinput.rules'

check deb docker.io/library/ubuntu:24.04 "${ROOT_ROUND_TRIP}"
check rpm registry.fedoraproject.org/fedora:40 "${ROOT_ROUND_TRIP}"
check tarball docker.io/library/archlinux:latest "${ROOT_ROUND_TRIP}"
check home-folder docker.io/library/ubuntu:24.04 '
    useradd -m user
    su user -c "NECTARLINK_ASSETS_DIR=/dist sh /dist/install.sh"
    su user -c "~/.local/bin/nectarlink-desktop --version" >/dev/null
    test -f /home/user/.local/share/applications/nectarlink.desktop
    su user -c "NECTARLINK_ASSETS_DIR=/dist sh /dist/install.sh --uninstall"
    ! test -e /home/user/.local/bin/nectarlink-desktop'
check bad-checksum docker.io/library/ubuntu:24.04 '
    mkdir /tampered && cp /dist/* /tampered/
    echo tampered >> /tampered/nectarlink_9.9.9_amd64.deb
    ! NECTARLINK_ASSETS_DIR=/tampered sh /tampered/install.sh
    ! command -v nectarlink-desktop'
echo "All install.sh checks passed."
