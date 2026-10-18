#!/usr/bin/env -S python3 -I
"""Tests for cooldown_check.py, using a fake index served from a temp directory."""

import importlib.util
import json
import tempfile
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "cooldown_check", Path(__file__).with_name("cooldown_check.py")
)
cc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cc)

NOW = datetime(2026, 10, 18, 12, 0, tzinfo=timezone.utc)
CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"


def lockfile(*packages):
    out = ["version = 4\n"]
    for name, version, source in packages:
        out.append(f'[[package]]\nname = "{name}"\nversion = "{version}"\n')
        if source:
            out.append(f'source = "{source}"\n')
    return "\n".join(out)


class CooldownCheckTest(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.tmp = Path(tmp.name)
        self.index = self.tmp / "index"
        self.index_url = self.index.as_uri()
        (self.tmp / "config.toml").write_text('[registry]\nglobal-min-publish-age = "7 days"\n')

    def publish(self, name, version, age):
        path = self.index / cc.index_path(name)
        path.parent.mkdir(parents=True, exist_ok=True)
        pubtime = (NOW - age).strftime("%Y-%m-%dT%H:%M:%SZ")
        with path.open("a") as f:
            f.write(json.dumps({"name": name, "vers": version, "pubtime": pubtime}) + "\n")

    def run_main(self, lock_text, exceptions=""):
        (self.tmp / "Cargo.lock").write_text(lock_text)
        (self.tmp / "exceptions.toml").write_text(exceptions)
        return cc.main([
            "--lockfile", str(self.tmp / "Cargo.lock"),
            "--config", str(self.tmp / "config.toml"),
            "--exceptions", str(self.tmp / "exceptions.toml"),
            "--cache", str(self.tmp / "cache.json"),
            "--index-url", self.index_url,
            "--now", NOW.isoformat(),
        ])

    def test_old_versions_pass_and_are_cached(self):
        self.publish("serde", "1.0.0", timedelta(days=30))
        self.assertEqual(self.run_main(lockfile(("serde", "1.0.0", CRATES_IO))), 0)
        cache = json.loads((self.tmp / "cache.json").read_text())
        self.assertIn("serde@1.0.0", cache)

    def test_young_version_blocks(self):
        self.publish("zip", "9.0.4", timedelta(days=10))
        self.publish("zip", "9.0.5", timedelta(days=1))
        self.assertEqual(self.run_main(lockfile(("zip", "9.0.5", CRATES_IO))), 1)

    def test_exactly_min_age_passes(self):
        self.publish("abc", "0.1.0", timedelta(days=7))
        self.assertEqual(self.run_main(lockfile(("abc", "0.1.0", CRATES_IO))), 0)

    def test_exception_allows_young_version(self):
        self.publish("zip", "9.0.5", timedelta(days=1))
        exceptions = (
            '[[exception]]\ncrate = "zip"\nversion = "9.0.5"\n'
            'reason = "security fix"\nadded = "2026-10-18"\n'
        )
        self.assertEqual(self.run_main(lockfile(("zip", "9.0.5", CRATES_IO)), exceptions), 0)

    def test_exception_without_reason_is_an_error(self):
        self.publish("zip", "9.0.5", timedelta(days=1))
        exceptions = '[[exception]]\ncrate = "zip"\nversion = "9.0.5"\nadded = "2026-10-18"\n'
        self.assertEqual(self.run_main(lockfile(("zip", "9.0.5", CRATES_IO)), exceptions), 2)

    def test_local_packages_are_ignored(self):
        self.assertEqual(self.run_main(lockfile(("armarium-core", "0.1.0", None))), 0)

    def test_git_source_blocks(self):
        lock = lockfile(("foo", "1.0.0", "git+https://example.com/foo#abc123"))
        self.assertEqual(self.run_main(lock), 1)

    def test_unreachable_index_blocks(self):
        self.assertEqual(self.run_main(lockfile(("missing", "1.0.0", CRATES_IO))), 2)

    def test_cached_versions_are_not_refetched(self):
        self.publish("serde", "1.0.0", timedelta(days=30))
        self.run_main(lockfile(("serde", "1.0.0", CRATES_IO)))
        (self.index / cc.index_path("serde")).unlink()
        self.assertEqual(self.run_main(lockfile(("serde", "1.0.0", CRATES_IO))), 0)

    def test_missing_config_setting_blocks(self):
        (self.tmp / "config.toml").write_text("")
        self.assertEqual(self.run_main(lockfile()), 2)


class HelpersTest(unittest.TestCase):
    def test_index_path(self):
        self.assertEqual(cc.index_path("a"), "1/a")
        self.assertEqual(cc.index_path("ab"), "2/ab")
        self.assertEqual(cc.index_path("abc"), "3/a/abc")
        self.assertEqual(cc.index_path("Serde"), "se/rd/serde")

    def test_parse_duration(self):
        self.assertEqual(cc.parse_duration("7 days"), timedelta(days=7))
        self.assertEqual(cc.parse_duration("1 day"), timedelta(days=1))
        self.assertEqual(cc.parse_duration("24 hours"), timedelta(hours=24))
        self.assertEqual(cc.parse_duration("2 weeks"), timedelta(weeks=2))
        self.assertEqual(cc.parse_duration("0"), timedelta(0))
        with self.assertRaises(cc.CheckError):
            cc.parse_duration("7 fortnights")


if __name__ == "__main__":
    unittest.main()
