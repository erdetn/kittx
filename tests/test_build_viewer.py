"""Exercise build selection without installing SDKs or compiling GPUI."""

import contextlib
import importlib.util
import io
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[1] / "build_viewer.py"
SPEC = importlib.util.spec_from_file_location("build_viewer", SCRIPT)
builder = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(builder)


class BuildViewerTests(unittest.TestCase):
    def test_os_selection_preserves_architecture(self):
        for architecture in ("x86_64", "aarch64"):
            host = f"{architecture}-apple-darwin"
            for operating_system, suffix in builder.OS_SUFFIXES.items():
                with self.subTest(architecture=architecture, os=operating_system):
                    self.assertEqual(
                        builder.select_target(operating_system, host),
                        f"{architecture}-{suffix}",
                    )

    def test_default_keeps_exact_host_including_linux_libc(self):
        for host in ("aarch64-apple-darwin", "x86_64-unknown-linux-musl", "x86_64-pc-windows-msvc"):
            self.assertEqual(builder.select_target(None, host), host)
        with self.assertRaises(ValueError):
            builder.select_target(None, "x86_64-unknown-freebsd")
        with self.assertRaises(ValueError):
            builder.select_target("windows", "riscv64gc-unknown-linux-gnu")

    def test_release_command_is_independent_of_working_directory(self):
        for args, target in (([], "aarch64-apple-darwin"), (["windows"], "aarch64-pc-windows-msvc"), (["linux"], "aarch64-unknown-linux-gnu"), (["macos"], "aarch64-apple-darwin")):
            with self.subTest(args=args), tempfile.TemporaryDirectory(prefix="viewer build ") as temporary:
                viewer = Path(temporary) / "project with spaces" / "src" / "viewerui"
                host = subprocess.CompletedProcess(["rustc"], 0, stdout="rustc 1.98\nhost: aarch64-apple-darwin\n")
                with patch.object(builder, "VIEWER_DIR", viewer), patch.object(builder.subprocess, "run", side_effect=[host, subprocess.CompletedProcess(["cargo"], 0)]) as run, contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(builder.main(args), 0)
                self.assertEqual(run.call_args_list[0].kwargs["cwd"], viewer)
                self.assertEqual(run.call_args_list[1].args[0], ["cargo", "build", "--locked", "--release", "--bin", "kittx-viewer", "--manifest-path", str(viewer / "Cargo.toml"), "--target", target])
                self.assertEqual(run.call_args_list[1].kwargs, {"cwd": viewer, "check": True})

    def test_failures_and_invalid_arguments_do_not_report_success(self):
        with patch.object(builder, "host_target", return_value="aarch64-apple-darwin"), patch.object(builder.subprocess, "run", side_effect=subprocess.CalledProcessError(101, ["cargo"])), contextlib.redirect_stdout(io.StringIO()) as output, contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(builder.main(["linux"]), 101)
            self.assertNotIn("completed", output.getvalue())
        with patch.object(builder, "host_target", side_effect=FileNotFoundError("rustc missing")), contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(builder.main([]), 1)
        with patch.object(builder, "host_target") as host, contextlib.redirect_stderr(io.StringIO()):
            for args in (["android"], ["linux", "windows"]):
                with self.assertRaises(SystemExit) as error:
                    builder.main(args)
                self.assertEqual(error.exception.code, 2)
            host.assert_not_called()

    def test_missing_host_line_is_rejected(self):
        with patch.object(builder.subprocess, "run", return_value=subprocess.CompletedProcess(["rustc"], 0, stdout="rustc 1.98\n")):
            with self.assertRaises(ValueError):
                builder.host_target()


if __name__ == "__main__":
    unittest.main()
