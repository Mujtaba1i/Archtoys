"""Tests for add_supporter.py: who gets listed, and how."""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(__file__))
import add_supporter as a  # noqa: E402

BASE = "# Supporters\n\n## Thank you\n\n" + a.MARK + " by the Action. -->\n"


class AddSupporter(unittest.TestCase):
    def test_a_public_supporter_is_added_with_their_message(self):
        new, note = a.add(BASE, "Sara", "Love it!", "m1")
        self.assertTrue(new.endswith("- **Sara**: “Love it\\!” <!-- kofi:m1 -->\n"))
        self.assertIn("Added Sara", note)

    def test_the_same_tip_twice_is_listed_once(self):
        new, _ = a.add(BASE, "Sara", "", "m1")
        again, note = a.add(new, "Sara", "", "m1")
        self.assertIsNone(again)
        self.assertIn("already listed", note)

    def test_a_kofi_test_adds_nobody(self):
        new, note = a.add(BASE, "Jo Example", "Good luck", "x", test=True)
        self.assertIsNone(new)
        self.assertIn("Nobody was added", note)

    def test_an_empty_name_adds_nobody(self):
        self.assertIsNone(a.add(BASE, "  \n ")[0])

    def test_names_cant_inject_links_html_or_new_lines(self):
        new, _ = a.add(BASE, "[click](http://x)<img src=x>\n- **fake**", "")
        line = new.splitlines()[-1]
        self.assertEqual(new.count("\n- "), 1)  # still one entry
        self.assertNotIn("](", line.replace("\\](", ""))
        self.assertIn("\\<img", line)

    def test_long_names_and_messages_are_cut(self):
        new, _ = a.add(BASE, "n" * 300, "m" * 900)
        line = new.splitlines()[-1]
        self.assertIn("n" * a.MAX_NAME + "**", line)
        self.assertNotIn("m" * (a.MAX_MESSAGE + 1), line)

    def test_a_strange_id_is_ignored_not_written(self):
        new, _ = a.add(BASE, "Sara", "", "--> <script>")
        self.assertNotIn("kofi:", new)

    def test_without_the_marker_it_says_so(self):
        new, note = a.add("# Supporters\n", "Sara")
        self.assertIsNone(new)
        self.assertIn("marker", note)


if __name__ == "__main__":
    unittest.main()
