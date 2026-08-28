import unittest
import xml.etree.ElementTree as ET

from scripts.mobile_cross_platform_runner import AndroidUI, RunnerError, parse_bounds


class RunnerHelpersTest(unittest.TestCase):
    def test_parse_bounds_returns_center(self) -> None:
        self.assertEqual(parse_bounds("[10,20][30,60]"), (20, 40))

    def test_parse_bounds_rejects_malformed_value(self) -> None:
        with self.assertRaises(RunnerError):
            parse_bounds("10,20,30,60")

    def test_clickable_ancestor_uses_semantic_parent(self) -> None:
        root = ET.fromstring(
            '<hierarchy><node clickable="true" bounds="[0,0][100,100]">'
            '<node text="Announce" clickable="false" bounds="[10,10][20,20]"/>'
            "</node></hierarchy>"
        )
        text = AndroidUI.find(root, lambda node: node.attrib.get("text") == "Announce")

        self.assertIsNotNone(text)
        self.assertEqual(AndroidUI.clickable_ancestor(root, text).attrib["bounds"], "[0,0][100,100]")


if __name__ == "__main__":
    unittest.main()
