"""Focused verifier checks; never build a wheel or install a package."""

from __future__ import annotations

import base64
import hashlib
import stat
import tomllib
import unittest
import zipfile

import wheel


class WheelAdmissionTests(unittest.TestCase):
    def test_unsafe_or_special_member_shapes(self) -> None:
        for name in (
            "/daml/file",
            "daml/../file",
            "daml//file",
            "daml/./file",
            "daml\\file",
            "daml/file\x00suffix",
            "C:/file",
        ):
            with self.subTest(name=name), self.assertRaises(ValueError):
                wheel.safe_member(zipfile.ZipInfo(name))
        link = zipfile.ZipInfo("daml/link")
        link.external_attr = (stat.S_IFLNK | 0o777) << 16
        with self.assertRaises(ValueError):
            wheel.safe_member(link)

    def test_record_accepts_valid_payload_and_rejects_tampering(self) -> None:
        content = b"authored payload"
        encoded = (
            base64.urlsafe_b64encode(hashlib.sha256(content).digest())
            .rstrip(b"=")
            .decode()
        )
        cargo = tomllib.loads((wheel.ROOT / "bindings/python/Cargo.toml").read_text())
        info = f"daml_config-{cargo['package']['version']}.dist-info"
        files = {
            "daml/__init__.py": content,
            info + "/RECORD": (
                f"daml/__init__.py,sha256={encoded},{len(content)}\n{info}/RECORD,,\n"
            ).encode(),
        }
        wheel.verify_record(files, info)
        files["daml/__init__.py"] = b"tampered payload"
        with self.assertRaises(ValueError):
            wheel.verify_record(files, info)


if __name__ == "__main__":
    unittest.main()
