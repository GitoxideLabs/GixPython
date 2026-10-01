import unittest

import gix


class PackageTests(unittest.TestCase):
    def test_build_identity(self):
        self.assertEqual(gix.__version__, "0.1.0")
        self.assertIn("parallel", gix.build_features())
        self.assertTrue({"sha1", "sha256"}.intersection(gix.build_features()))
        self.assertEqual(len(gix.__gix_revision__), 40)
        self.assertTrue(issubclass(gix.FeatureUnavailableError, gix.Error))


if __name__ == "__main__":
    unittest.main()
