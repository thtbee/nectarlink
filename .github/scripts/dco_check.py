# SPDX-License-Identifier: GPL-3.0-or-later
"""Fails if a pull request has a commit without a Developer Certificate of Origin
sign-off (CONTRIBUTING.md). Reads the commits from stdin, one JSON object a line, as
`gh api repos/OWNER/REPO/pulls/N/commits --paginate --jq '.[]'` prints them."""
import json
import re
import sys

SIGN_OFF = re.compile(r"^Signed-off-by:\s*.+<([^<>\s]+)>\s*$", re.MULTILINE | re.IGNORECASE)


def problem(item):
    """Why this commit fails the check, or None if it passes."""
    # GitHub makes merge commits itself (Update branch), and bots certify nothing.
    if len(item.get("parents") or []) > 1:
        return None
    author = item.get("author") or {}
    if author.get("type") == "Bot" or str(author.get("login", "")).endswith("[bot]"):
        return None
    commit = item["commit"]
    signed = {email.lower() for email in SIGN_OFF.findall(commit["message"])}
    if not signed:
        return "has no Signed-off-by line"
    people = {commit[who]["email"].lower() for who in ("author", "committer")}
    if not signed & people:
        return f"is signed off by {', '.join(sorted(signed))}, not its author ({commit['author']['email']})"
    return None


def main():
    commits = [json.loads(line) for line in sys.stdin if line.strip()]
    if not commits:
        print("::error::Found no commits to check.")
        return 1
    failed = 0
    for item in commits:
        why = problem(item)
        if why:
            failed += 1
            subject = item["commit"]["message"].splitlines()[0]
            print(f"::error::{item['sha'][:7]} {why}: {subject}")
    if failed:
        print(
            f"{failed} of {len(commits)} commits need a sign-off. To add it to every commit "
            "on your branch and update the pull request:\n\n"
            "    git rebase --signoff origin/main\n"
            "    git push --force-with-lease\n\n"
            "What signing off means: see DCO.txt and CONTRIBUTING.md."
        )
        return 1
    print(f"All {len(commits)} commits are signed off.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
