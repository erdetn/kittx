"""Test release builds and launching with fake tools; no GPUI, SDKs, or GUI needed."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "build_probe.sh"


@unittest.skipUnless(os.name == "posix", "Fake-tool fixtures require a POSIX host")
class BuildProbeTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="probe build ")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.project = self.root / "project with spaces"
        self.package = self.project / "src" / "kittxprobe"
        self.package.mkdir(parents=True)
        self.script = self.project / SCRIPT.name
        shutil.copy2(SCRIPT, self.script)
        tools = self.root / "fake tools"
        tools.mkdir()
        for name, contents in {
            "rustc": (
                '#!/bin/sh\n'
                'printf "%s\\n" "$PWD" "$@" > "$BUILD_TEST_RUSTC_LOG"\n'
                'printf "%s\\n" "$BUILD_TEST_COMPILER_INFO"\n'
                'exit "$BUILD_TEST_RUSTC_STATUS"\n'
            ),
            "cargo": (
                '#!/bin/sh\n'
                'case "$1" in\n'
                'build) printf "%s\\n" "$PWD" "$@" > "$BUILD_TEST_BUILD_LOG"\n'
                '       exit "$BUILD_TEST_BUILD_STATUS" ;;\n'
                'run) test -f "$BUILD_TEST_BUILD_LOG" || exit 99\n'
                '     printf "%s\\n" "$PWD" "$@" > "$BUILD_TEST_RUN_LOG"\n'
                '     exit "$BUILD_TEST_RUN_STATUS" ;;\n'
                'clean) printf "%s\\n" "$PWD" "$@" > "$BUILD_TEST_CLEAN_LOG"\n'
                '       exit "$BUILD_TEST_CLEAN_STATUS" ;;\n'
                '*) exit 98 ;;\n'
                'esac\n'
            ),
        }.items():
            tool = tools / name
            tool.write_text(contents)
            tool.chmod(0o755)
        self.compiler_log = self.root / "rustc.log"
        self.build_log = self.root / "build.log"
        self.run_log = self.root / "run.log"
        self.clean_log = self.root / "clean.log"
        self.env = dict(
            os.environ,
            PATH=str(tools) + os.pathsep + os.environ.get("PATH", ""),
            BUILD_TEST_RUSTC_LOG=str(self.compiler_log),
            BUILD_TEST_BUILD_LOG=str(self.build_log),
            BUILD_TEST_RUN_LOG=str(self.run_log),
            BUILD_TEST_CLEAN_LOG=str(self.clean_log),
            BUILD_TEST_COMPILER_INFO="rustc 1.98.1\nhost: aarch64-apple-darwin\n",
            BUILD_TEST_RUSTC_STATUS="0",
            BUILD_TEST_BUILD_STATUS="0",
            BUILD_TEST_RUN_STATUS="0",
            BUILD_TEST_CLEAN_STATUS="0",
        )

    def run_builder(self, *args, host=None, **environment):
        for log in (self.compiler_log, self.build_log, self.run_log, self.clean_log):
            log.unlink(missing_ok=True)
        env = dict(self.env, **environment)
        if host is not None:
            env["BUILD_TEST_COMPILER_INFO"] = f"rustc 1.98.1\nhost: {host}\n"
        return subprocess.run(
            [str(self.script), *args],
            cwd=self.root,
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )

    def cargo_arguments(self, command, target):
        return [
            str(self.package), command, "--locked", "--release", "--bin",
            "KittxProbe", "--manifest-path", str(self.package / "Cargo.toml"),
            "--target", target,
        ]

    def assert_build(self, result, target, runs):
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.compiler_log.read_text().splitlines(), [str(self.package), "-vV"])
        self.assertEqual(self.build_log.read_text().splitlines(), self.cargo_arguments("build", target))
        self.assertIn(f"release build completed for {target}", result.stdout)
        self.assertFalse(self.clean_log.exists())
        if runs:
            self.assertEqual(self.run_log.read_text().splitlines(), self.cargo_arguments("run", target))
            self.assertIn("Starting KittxProbe", result.stdout)
        else:
            self.assertFalse(self.run_log.exists())
            self.assertNotIn("Starting KittxProbe", result.stdout)

    def test_default_preserves_exact_host_and_runs_after_build(self):
        for host in (
            "aarch64-apple-darwin", "x86_64-pc-windows-msvc",
            "x86_64-pc-windows-gnu", "x86_64-unknown-linux-gnu",
            "aarch64-unknown-linux-musl",
        ):
            with self.subTest(host=host):
                self.assert_build(self.run_builder(host=host), host, runs=True)

    def test_os_selection_retains_architecture_and_runs_only_exact_host(self):
        for architecture in ("x86_64", "aarch64"):
            for operating_system, suffix in (
                ("windows", "pc-windows-msvc"),
                ("linux", "unknown-linux-gnu"),
                ("macos", "apple-darwin"),
            ):
                host = f"{architecture}-apple-darwin"
                target = f"{architecture}-{suffix}"
                with self.subTest(architecture=architecture, os=operating_system):
                    result = self.run_builder(operating_system, host=host)
                    self.assert_build(result, target, runs=target == host)
                    if target != host:
                        self.assertIn("run this build on the target platform", result.stdout)

    def test_build_only_option_in_either_position_skips_launch(self):
        for arguments in (("--build-only",), ("macos", "--build-only"), ("--build-only", "macos")):
            with self.subTest(arguments=arguments):
                result = self.run_builder(*arguments)
                self.assert_build(result, "aarch64-apple-darwin", runs=False)
                self.assertIn("Build only", result.stdout)

    def test_same_os_with_different_libc_or_abi_is_not_automatically_run(self):
        for operating_system, host, target in (
            ("linux", "aarch64-unknown-linux-musl", "aarch64-unknown-linux-gnu"),
            ("windows", "x86_64-pc-windows-gnu", "x86_64-pc-windows-msvc"),
        ):
            with self.subTest(host=host):
                self.assert_build(self.run_builder(operating_system, host=host), target, runs=False)

    def test_help_and_invalid_arguments_do_not_invoke_tools(self):
        for arguments, status in (
            (("--help",), 0), (("-h",), 0), (("android",), 2),
            (("linux", "windows"), 2), (("--invalid",), 2),
        ):
            with self.subTest(arguments=arguments):
                result = self.run_builder(*arguments)
                self.assertEqual(result.returncode, status)
                self.assertIn("Usage:", result.stdout + result.stderr)
                self.assertFalse(self.compiler_log.exists())
                self.assertFalse(self.build_log.exists())
                self.assertFalse(self.run_log.exists())

    def test_invalid_compiler_output_or_unsupported_targets_do_not_build(self):
        for arguments, environment in (
            ((), {"BUILD_TEST_COMPILER_INFO": "rustc 1.98.1"}),
            ((), {"host": "x86_64-unknown-freebsd"}),
            ((), {"host": "aarch64-linux-android"}),
            (("windows",), {"host": "riscv64gc-unknown-linux-gnu"}),
        ):
            with self.subTest(arguments=arguments, environment=environment):
                result = self.run_builder(*arguments, **environment)
                self.assertNotEqual(result.returncode, 0)
                self.assertTrue(result.stderr)
                self.assertFalse(self.build_log.exists())
                self.assertFalse(self.run_log.exists())
                self.assertNotIn("completed", result.stdout)

    def test_crlf_compiler_output_is_supported(self):
        result = self.run_builder(BUILD_TEST_COMPILER_INFO="rustc 1.98.1\r\nhost: aarch64-apple-darwin\r\n")
        self.assert_build(result, "aarch64-apple-darwin", runs=True)

    def test_compiler_and_build_failures_never_launch(self):
        result = self.run_builder(BUILD_TEST_RUSTC_STATUS="17")
        self.assertEqual(result.returncode, 17)
        self.assertFalse(self.build_log.exists())
        self.assertFalse(self.run_log.exists())
        result = self.run_builder(BUILD_TEST_BUILD_STATUS="101")
        self.assertEqual(result.returncode, 101)
        self.assertIn("Build failed", result.stderr)
        self.assertFalse(self.run_log.exists())
        self.assertNotIn("completed", result.stdout)

    def test_application_exit_status_is_propagated(self):
        result = self.run_builder(BUILD_TEST_RUN_STATUS="23")
        self.assertEqual(result.returncode, 23)
        self.assertTrue(self.build_log.exists())
        self.assertTrue(self.run_log.exists())
        self.assertIn("Starting KittxProbe", result.stdout)

    def test_clean_uses_probe_manifest_without_building_running_or_detecting_host(self):
        result = self.run_builder("clean", BUILD_TEST_RUSTC_STATUS="17")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.clean_log.read_text().splitlines(), [
            str(self.package), "clean", "--manifest-path", str(self.package / "Cargo.toml"),
        ])
        self.assertIn("Cleaning KittxProbe", result.stdout)
        self.assertFalse(self.compiler_log.exists())
        self.assertFalse(self.build_log.exists())
        self.assertFalse(self.run_log.exists())

    def test_clean_failure_is_propagated_without_building_or_running(self):
        result = self.run_builder("clean", BUILD_TEST_CLEAN_STATUS="19")
        self.assertEqual(result.returncode, 19)
        self.assertTrue(self.clean_log.exists())
        self.assertFalse(self.compiler_log.exists())
        self.assertFalse(self.build_log.exists())
        self.assertFalse(self.run_log.exists())

    def test_clean_rejects_combined_arguments_before_invoking_tools(self):
        for arguments in (
            ("clean", "windows"), ("linux", "clean"),
            ("clean", "--build-only"), ("--build-only", "clean"), ("clean", "clean"),
        ):
            with self.subTest(arguments=arguments):
                result = self.run_builder(*arguments)
                self.assertEqual(result.returncode, 2)
                self.assertIn("Use clean by itself", result.stderr)
                for log in (self.compiler_log, self.build_log, self.run_log, self.clean_log):
                    self.assertFalse(log.exists())


if __name__ == "__main__":
    unittest.main()
