#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# rustc supplies its bundled LLD search path for x86_64-unknown-linux-gnu.
set -eu

if command -v mold >/dev/null 2>&1; then
    # Last option wins over rustc's default -fuse-ld=lld.
    exec cc "$@" -fuse-ld=mold
fi

exec cc "$@" -fuse-ld=lld
