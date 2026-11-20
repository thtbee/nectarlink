# SPDX-License-Identifier: GPL-3.0-or-later
import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location(
    "discord_post", pathlib.Path(__file__).with_name("discord-post.py")
)
discord_post = importlib.util.module_from_spec(spec)
spec.loader.exec_module(discord_post)


class DiscordPost(unittest.TestCase):
    def test_changelog_entries_become_their_bold_lead(self):
        notes = "- **Faster photos.** The grid scrolls smoothly now (#12). Thanks @x.\n"
        self.assertEqual(discord_post.points(notes), (["Faster photos."], 0))

    def test_nightly_lines_lose_their_references(self):
        notes = "- Keep pop-ups paused after a restart (#440)\n- Fix the start (#466, #467)\n"
        self.assertEqual(
            discord_post.points(notes)[0],
            ["Keep pop-ups paused after a restart", "Fix the start"],
        )

    def test_only_the_top_changes_then_a_count(self):
        notes = "".join(f"- change {i}\n" for i in range(11))
        shown, more = discord_post.points(notes)
        self.assertEqual(len(shown), discord_post.MAX_POINTS)
        self.assertEqual(more, 3)
        self.assertIn("-# and 3 more", discord_post.message("h", "l", notes, "")["content"])

    def test_only_the_role_can_be_pinged(self):
        msg = discord_post.message("Nightly 3", "https://x", "- hi @everyone\n", "123")
        self.assertEqual(msg["allowed_mentions"], {"parse": [], "roles": ["123"]})
        self.assertTrue(msg["content"].startswith("<@&123>\n## Nightly 3\n"))
        self.assertNotIn("@everyone", msg["content"])

    def test_no_role_pings_nobody(self):
        msg = discord_post.message("h", "https://x", "", "")
        self.assertEqual(msg["allowed_mentions"], {"parse": [], "roles": []})
        self.assertEqual(msg["content"], "## h\n\nhttps://x")

    def test_it_fits_in_one_message_with_the_link(self):
        notes = "".join(f"- {'x' * 300}\n" for _ in range(8))
        content = discord_post.message("h" * 1900, "https://link", notes, "1")["content"]
        self.assertLessEqual(len(content), 2000)
        self.assertTrue(content.endswith("https://link"))


if __name__ == "__main__":
    unittest.main()
