"""runtime/server/preflight.sh prints one JSON object the app can parse."""
import json
import os
import platform
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "..", "server", "preflight.sh")
KEYS = {"arch", "os_id", "os_pretty", "disk_free_mb", "mem_mb", "systemd_user", "linger", "apt",
        "sudo_nopass", "docker_access", "installed", "version", "tools"}
TOOLS = {"Xvfb", "x11vnc", "websockify", "novnc", "python3_venv", "curl", "xz", "xclip", "docker", "compose"}


def run(home):
    env = dict(os.environ, HOME=home)
    out = subprocess.run(["bash", SCRIPT], env=env, capture_output=True, text=True, timeout=60)
    return out


class PreflightTest(unittest.TestCase):
    def test_json_shape(self):
        with tempfile.TemporaryDirectory() as home:
            out = run(home)
            self.assertEqual(out.returncode, 0, out.stderr)
            d = json.loads(out.stdout)
            self.assertEqual(set(d), KEYS)
            self.assertEqual(set(d["tools"]), TOOLS)
            self.assertIs(d["installed"], False)
            self.assertIsNone(d["version"])
            self.assertEqual(d["arch"], platform.machine())
            self.assertIsInstance(d["disk_free_mb"], int)
            self.assertIsInstance(d["mem_mb"], int)
            for k in ("systemd_user", "linger", "apt", "sudo_nopass", "docker_access"):
                self.assertIsInstance(d[k], bool, k)

    def test_installed_and_version(self):
        with tempfile.TemporaryDirectory() as home:
            os.makedirs(os.path.join(home, "siliconmetatrader5", "runtime"))
            with open(os.path.join(home, "siliconmetatrader5", "VERSION"), "w") as f:
                f.write('abc"123\n')
            d = json.loads(run(home).stdout)
            self.assertIs(d["installed"], True)
            self.assertEqual(d["version"], 'abc"123')


if __name__ == "__main__":
    unittest.main()
