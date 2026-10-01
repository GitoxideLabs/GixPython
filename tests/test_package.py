import unittest
import sys
import sysconfig

import gix


class PackageTests(unittest.TestCase):
    @unittest.skipUnless(sysconfig.get_config_var("Py_GIL_DISABLED"), "requires free-threaded CPython")
    def test_import_keeps_the_gil_disabled(self):
        self.assertFalse(sys._is_gil_enabled())

    def test_build_identity(self):
        self.assertEqual(gix.__version__, "0.1.0")
        self.assertIn("parallel", gix.build_features())
        self.assertTrue({"sha1", "sha256"}.intersection(gix.build_features()))
        self.assertEqual(len(gix.__gix_revision__), 40)
        self.assertTrue(issubclass(gix.FeatureUnavailableError, gix.Error))


if __name__ == "__main__":
    unittest.main()
