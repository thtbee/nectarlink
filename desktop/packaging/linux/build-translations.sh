#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Compile the desktop's offered languages into a package staging directory.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
DEST="${1:?usage: build-translations.sh OUTPUT_DIRECTORY}"

LRELEASE="${LRELEASE:-}"
if [[ -z "${LRELEASE}" ]]; then
    for candidate in "${QT_HOST_PATH:-}/bin/lrelease" "${QT_ROOT_DIR:-}/bin/lrelease" \
        "$(dirname "${QMAKE:-/nonexistent/qmake}")/lrelease" \
        /usr/lib/qt6/bin/lrelease lrelease6 lrelease; do
        if command -v "${candidate}" >/dev/null 2>&1; then
            LRELEASE="${candidate}"
            break
        fi
    done
fi
if [[ -z "${LRELEASE}" ]]; then
    echo "error: lrelease is required; install qt6-l10n-tools or set LRELEASE" >&2
    exit 1
fi

# Use the same list as Settings. Fail rather than silently ship an offered
# language without its catalogue, and keep incomplete languages out of packages.
CATALOGUES="$(python3 - "${REPO_ROOT}" <<'PY'
import pathlib
import re
import sys
import xml.etree.ElementTree as ET

root = pathlib.Path(sys.argv[1]) / "desktop/app"
source = (root / "src/i18n.rs").read_text(encoding="utf-8")
languages = source.split("pub const LANGUAGES:", 1)[1].split("];", 1)[0]
for code in re.findall(r'\(\s*"([^"]*)"\s*,', languages):
    if code in ("", "en"):
        continue
    path = root / "i18n" / f"nectarlink_{code}.ts"
    tree = ET.parse(path)
    for message in tree.findall(".//message"):
        translation = message.find("translation")
        if translation is None or translation.get("type") == "unfinished":
            sys.exit(f"error: incomplete offered language: {path}")
    print(path)
PY
)"

mkdir -p "${DEST}"
while IFS= read -r catalogue; do
    [[ -n "${catalogue}" ]] || continue
    "${LRELEASE}" "${catalogue}" -qm "${DEST}/$(basename "${catalogue}" .ts).qm"
done <<< "${CATALOGUES}"
