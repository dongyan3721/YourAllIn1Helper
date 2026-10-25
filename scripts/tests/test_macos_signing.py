"""签名策略回归：无身份不得降级，验证签名必须绑定证书而非单一构建哈希。"""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("macos_signing", Path(__file__).parents[1] / "macos_signing.py")
signing = importlib.util.module_from_spec(spec)
spec.loader.exec_module(signing)


class SigningPolicyTests(unittest.TestCase):
    def test_no_identity_and_explicit_adhoc_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch.object(signing, "CONFIG", Path(directory) / "missing.json"):
                for value in ("", "-"):
                    with patch.dict(os.environ, {"MACOS_SIGNING_IDENTITY": value}):
                        with self.assertRaises(RuntimeError):
                            signing.identity()

    def test_existing_identity_is_reused_without_regenerating_keys(self):
        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory) / "identity.json"
            config.write_text(json.dumps({"sha1": "A" * 40}))
            with patch.object(signing, "CONFIG", config), patch.dict(os.environ, {"MACOS_SIGNING_IDENTITY": ""}):
                with patch.object(signing, "run") as command:
                    signing.setup_local()
                    command.assert_not_called()
                self.assertEqual(signing.identity(), "A" * 40)

    def test_adhoc_or_identifier_only_signature_cannot_publish_manifest(self):
        for signature in ('Signature=adhoc\n# designated => cdhash H"abcd"',
                          'Signature=certificate\n# designated => identifier "com.allinone.desktop"'):
            with tempfile.TemporaryDirectory() as directory:
                app = Path(directory) / "Example.app"
                with patch.object(signing, "identity", return_value="fixed identity"), patch.object(
                    signing, "run", return_value=subprocess.CompletedProcess([], 0, "", signature)
                ):
                    with self.assertRaises(RuntimeError):
                        signing.sign(app)
                self.assertFalse((app.parent / "macos-build.json").exists())


if __name__ == "__main__":
    unittest.main()
