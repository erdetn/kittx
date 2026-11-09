"""Exercise launcher packaging without a compiler or desktop environment."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


@unittest.skipUnless(os.name == "posix", "Fake Cargo uses a POSIX shell")
class BuildLauncherTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="kittx launcher ")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.script = self.root / "build_launcher.sh"
        shutil.copy2(Path(__file__).resolve().parents[1] / self.script.name, self.script)
        tools = self.root / "tools"
        tools.mkdir()
        cargo = tools / "cargo"
        cargo.write_text(
            '#!/bin/sh\n'
            'printf "%s\\n" "$*" >> "$LAUNCHER_TEST_LOG"\n'
            'case "$*" in *"$LAUNCHER_TEST_FAIL"*) exit 9 ;; esac\n'
        )
        cargo.chmod(0o755)
        self.log = self.root / "calls"
        self.env = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ["PATH"],
                        LAUNCHER_TEST_LOG=str(self.log), LAUNCHER_TEST_FAIL="never-match")
        self.env.pop("CARGO_TARGET_DIR", None)

    def run_script(self, *arguments, **environment):
        return subprocess.run([str(self.script), *arguments], cwd=self.root.parent,
                              env=dict(self.env, **environment), text=True,
                              capture_output=True, check=False)

    def test_integrated_launcher_build_does_not_build_standalone_executables(self):
        self.assertEqual(self.run_script().returncode, 0)
        lines = self.log.read_text().splitlines()
        self.assertEqual(len(lines), 2)
        self.assertTrue(lines[0].startswith("build --locked --release"))
        self.assertIn("--manifest-path src/kittxlauncher/Cargo.toml", lines[0])
        self.assertIn(f"--target-dir {self.root}/src/kittxlauncher/target", lines[0])
        self.assertIn("--bin KittxLauncher", lines[0])
        self.assertTrue(lines[1].startswith("run --locked --release"))
        self.assertIn("--bin KittxLauncher", lines[1])

    def test_build_only_and_custom_target_directory(self):
        self.assertEqual(self.run_script("--build-only", CARGO_TARGET_DIR="output with spaces").returncode, 0)
        lines = self.log.read_text().splitlines()
        self.assertEqual(len(lines), 1)
        self.assertTrue(all("--target-dir output with spaces" in line for line in lines))

    def test_build_failure_stops_before_launching(self):
        self.assertEqual(self.run_script(LAUNCHER_TEST_FAIL="kittxlauncher").returncode, 9)
        self.assertEqual(len(self.log.read_text().splitlines()), 1)

    def test_bad_arguments_do_not_build(self):
        self.assertEqual(self.run_script("bad-option").returncode, 2)
        self.assertEqual(self.run_script("--build-only", "extra").returncode, 2)
        self.assertFalse(self.log.exists())


if __name__ == "__main__":
    unittest.main()
