# SPDX-License-Identifier: GPL-3.0-or-later
import unittest

from dco_check import problem

ME = "me@example.com"


def commit(message, author=ME, committer=ME, parents=1, login="me", kind="User"):
    return {
        "sha": "0123456789",
        "parents": [{}] * parents,
        "author": {"login": login, "type": kind},
        "commit": {
            "message": message,
            "author": {"email": author},
            "committer": {"email": committer},
        },
    }


class Problem(unittest.TestCase):
    def test_signed_by_the_author_passes(self):
        self.assertIsNone(problem(commit(f"fix: a\n\nSigned-off-by: Me <{ME}>")))

    def test_email_case_doesnt_matter(self):
        self.assertIsNone(problem(commit("fix: a\n\nSigned-off-by: Me <ME@Example.com>")))

    def test_signed_by_the_committer_passes(self):
        message = "fix: a\n\nSigned-off-by: Them <them@example.com>"
        self.assertIsNone(problem(commit(message, author=ME, committer="them@example.com")))

    def test_one_of_several_sign_offs_is_enough(self):
        message = f"fix: a\n\nSigned-off-by: Them <them@example.com>\nSigned-off-by: Me <{ME}>"
        self.assertIsNone(problem(commit(message)))

    def test_no_sign_off_fails(self):
        self.assertIn("no Signed-off-by", problem(commit("fix: a")))

    def test_someone_elses_sign_off_fails(self):
        self.assertIn("not its author", problem(commit("fix: a\n\nSigned-off-by: Them <them@example.com>")))

    def test_a_line_only_mentioning_it_fails(self):
        self.assertIn("no Signed-off-by", problem(commit(f"fix: a\n\nAdd Signed-off-by: Me <{ME}> later")))

    def test_a_sign_off_without_an_email_fails(self):
        self.assertIn("no Signed-off-by", problem(commit("fix: a\n\nSigned-off-by: Me")))

    def test_merge_commits_are_skipped(self):
        self.assertIsNone(problem(commit("Merge branch main", parents=2)))

    def test_bots_are_skipped(self):
        self.assertIsNone(problem(commit("chore: bump", login="dependabot[bot]", kind="Bot")))

    def test_an_unlinked_author_is_still_checked(self):
        item = commit("fix: a")
        item["author"] = None
        self.assertIn("no Signed-off-by", problem(item))


if __name__ == "__main__":
    unittest.main()
