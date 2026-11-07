"""Keep the startup DPI contract explicit and embedded for both resource compilers."""
from pathlib import Path
import unittest
import xml.etree.ElementTree as ET


class WindowsManifestTests(unittest.TestCase):
    def test_manifest_has_only_requested_dpi_and_os_contract(self):
        root_dir = Path(__file__).resolve().parents[1]
        resources = root_dir / "resources"
        root = ET.parse(resources / "shotkit.manifest").getroot()
        self.assertEqual(root.tag, "{urn:schemas-microsoft-com:asm.v1}assembly")
        self.assertEqual(list(root)[0].tag, "{urn:schemas-microsoft-com:asm.v1}assemblyIdentity")
        self.assertEqual(
            list(root)[0].attrib,
            {"type": "win32", "name": "BomBastikDE.ShotKit", "version": "1.0.0.0"},
        )
        settings = root.find(
            "{urn:schemas-microsoft-com:asm.v3}application/"
            "{urn:schemas-microsoft-com:asm.v3}windowsSettings"
        )
        self.assertIsNotNone(settings)
        self.assertEqual(
            {entry.tag: entry.text for entry in settings},
            {
                "{http://schemas.microsoft.com/SMI/2005/WindowsSettings}dpiAware": "true/pm",
                "{http://schemas.microsoft.com/SMI/2016/WindowsSettings}dpiAwareness": "PerMonitorV2",
            },
        )
        supported = root.findall(
            "{urn:schemas-microsoft-com:compatibility.v1}compatibility/"
            "{urn:schemas-microsoft-com:compatibility.v1}application/"
            "{urn:schemas-microsoft-com:compatibility.v1}supportedOS"
        )
        self.assertEqual(
            [entry.attrib for entry in supported],
            [{"Id": "{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"}],
        )
        # No Common Controls v6 or other activation-context changes in this fix.
        self.assertFalse(root.findall("{urn:schemas-microsoft-com:asm.v1}dependency"))
        self.assertIn('1 24 "shotkit.manifest"', (resources / "shotkit.rc").read_text())
        self.assertIn(
            "cargo:rerun-if-changed=resources/shotkit.manifest",
            (root_dir / "build.rs").read_text(),
        )


if __name__ == "__main__":
    unittest.main()
