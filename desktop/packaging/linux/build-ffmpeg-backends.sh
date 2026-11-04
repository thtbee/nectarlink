#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Build ABI adapters for the FFmpeg versions used by supported distributions.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
stage="$(mktemp -d -t nectarlink-ffmpeg-XXXXXX)"
trap 'rm -rf "${stage}"' EXIT
out="${FFMPEG_BACKEND_DIR:-${root}/target/release}"
mkdir -p "${out}"
cache="${FFMPEG_SOURCE_CACHE:-${stage}}"
mkdir -p "${cache}"

build_adapter() {
    local version="$1" checksum="$2" major="${1%%.*}"
    local archive="${cache}/ffmpeg-${version}.tar.xz"
    if [[ ! -f "${archive}" ]] || ! echo "${checksum}  ${archive}" | sha256sum -c --status; then
        curl -fsSL --retry 3 --retry-all-errors --connect-timeout 15 \
            "https://ffmpeg.org/releases/ffmpeg-${version}.tar.xz" -o "${archive}.tmp" ||
        curl -fsSL --retry 3 --retry-all-errors --connect-timeout 15 \
            "https://ftp.osuosl.org/pub/blfs/conglomeration/ffmpeg/ffmpeg-${version}.tar.xz" -o "${archive}.tmp"
        echo "${checksum}  ${archive}.tmp" | sha256sum -c -
        mv "${archive}.tmp" "${archive}"
    fi
    tar -xf "${archive}" -C "${stage}"
    (
        cd "${stage}/ffmpeg-${version}"
        ./configure --disable-everything --disable-x86asm --disable-programs --disable-doc >/dev/null
    )
    ${CXX:-c++} -std=c++17 -O2 -fPIC -shared -Wl,-Bsymbolic \
        -I"${stage}/ffmpeg-${version}" "${root}/desktop/app/cpp/linux_video_codec.cpp" -ldl \
        -o "${out}/libnectarlink-video-ffmpeg${major}.so"
}

build_adapter 7.1.1 733984395e0dbbe5c046abda2dc49a5544e7e0e1e2366bba849222ae9e3a03b1
build_adapter 8.0.1 05ee0b03119b45c0bdb4df654b96802e909e0a752f72e4fe3794f487229e5a41
build_adapter 9.0.2 8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e
