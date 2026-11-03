#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Reads changed paths, one per line, and succeeds when there are some and all
# of them are docs: no build, test or package uses them. CI skips its builds
# for such a change and a nightly doesn't count it as new. docs/design holds
# tokens.json, which the apps compile in, so it isn't docs here; only the
# root LICENSE goes into the packages, not LICENSES/.
set -euo pipefail

any=false
while IFS= read -r path; do
    [ -n "$path" ] || continue
    any=true
    case "$path" in
        docs/design/*) exit 1 ;;
        docs/* | LICENSES/* | .github/ISSUE_TEMPLATE/* | *.md) ;;
        *) exit 1 ;;
    esac
done
$any
