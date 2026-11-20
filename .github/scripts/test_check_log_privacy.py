# SPDX-License-Identifier: GPL-3.0-or-later
import unittest

from check_log_privacy import problems, without_tests


def check(source):
    return list(problems("x.rs", source))


class CheckLogPrivacyTest(unittest.TestCase):
    def test_fixed_messages_and_short_ids_pass(self):
        self.assertEqual(check('tracing::warn!(peer = %peer.short(), error = %e, "can\'t send");'), [])
        self.assertEqual(check('warn!(fps = format!("{:.1}", x), "stats");'), [])
        self.assertEqual(check('info!("literal {{braces}} are fine");'), [])

    def test_a_value_in_the_message_fails(self):
        (p,) = check('tracing::warn!("can\'t open {path}");')
        self.assertIn("x.rs:1", p)
        self.assertEqual(len(check('warn!(error = %e, "for {}", name);')), 1)

    def test_whole_device_ids_fail(self):
        self.assertEqual(len(check('debug!(%peer, "hi");')), 1)
        self.assertEqual(len(check('debug!(device = %d.id, "hi");')), 1)
        self.assertEqual(check('debug!(device = %d.id.short(), "hi");'), [])
        self.assertEqual(check('debug!(peer = %diag::peer(&id), "hi");'), [])

    def test_a_reason_lets_a_line_through(self):
        self.assertEqual(check('// log-ok: fixed lines\ninfo!("{message}");'), [])

    def test_multi_line_macros_report_their_first_line(self):
        (p,) = check('\n\nwarn!(\n    %peer,\n    "x"\n);')
        self.assertIn("x.rs:3", p)

    def test_unit_tests_arent_checked(self):
        source = 'fn a() {}\n#[cfg(test)]\nmod tests {\n    fn b() { warn!("{x}"); }\n}\n'
        self.assertNotIn("warn!", without_tests(source))


if __name__ == "__main__":
    unittest.main()
