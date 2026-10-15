"""Regression coverage for the hardware-approved classic factory Base layouts."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/check_ergohaven_profile.sh"
START = "python3 - <<'PY' || fail \"projected factory layers must mirror their K:04 Series references\"\n"


class ApprovedFactoryLayoutTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=os.environ.get("TMPDIR"))
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for source in (ROOT / "keyboards").glob("*/keyboard*.toml"):
            target = self.root / source.relative_to(ROOT)
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(source.read_bytes())
        self.code = SCRIPT.read_text().split(START, 1)[1].split("\nPY", 1)[0]

    def run_contract(self):
        return subprocess.run(
            ["python3", "-"], input=self.code, cwd=self.root,
            text=True, capture_output=True,
        )

    def test_approved_factory_profiles_pass_projection(self):
        result = self.run_contract()
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_unapproved_base_change_is_rejected(self):
        path = self.root / "keyboards/k03/keyboard.toml"
        source = path.read_text()
        self.assertIn("LGui    LAlt  LCtrl", source)
        path.write_text(source.replace("LGui    LAlt  LCtrl", "LShift  LAlt  LCtrl", 1))
        result = self.run_contract()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("keyboards/k03/keyboard.toml", result.stderr)

    def test_non_base_drift_is_rejected(self):
        path = self.root / "keyboards/classic_qube/keyboard_imperial44.toml"
        source = path.read_text()
        self.assertIn("Home", source)
        path.write_text(source.replace("Home", "End", 1))
        result = self.run_contract()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("keyboards/classic_qube/keyboard_imperial44.toml", result.stderr)


if __name__ == "__main__":
    unittest.main()
