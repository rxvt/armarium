#!/usr/bin/env -S python3 -I
"""Block Cargo.lock entries published more recently than the cooldown allows.

Second line of defence behind Cargo's own `registry.global-min-publish-age`:
catches an older Cargo that ignores the setting, or a forgotten
`CARGO_RESOLVER_INCOMPATIBLE_PUBLISH_AGE=allow`. Standard library only.
See docs/v1-plan.md#cooldown-nothing-younger-than-7-days.
"""

import argparse
import json
import re
import subprocess
import sys
import tomllib
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timedelta, timezone
from pathlib import Path

CRATES_IO_SOURCES = {
    "registry+https://github.com/rust-lang/crates.io-index",
    "sparse+https://index.crates.io/",
}
DEFAULT_INDEX_URL = "https://index.crates.io"
USER_AGENT = "armarium-cooldown-check (https://github.com/rxvt)"

# Months are counted as 31 days so the hook is never looser than Cargo.
UNIT_SECONDS = {
    "second": 1,
    "minute": 60,
    "hour": 3600,
    "day": 86400,
    "week": 7 * 86400,
    "month": 31 * 86400,
}


class CheckError(Exception):
    """A problem that means the check cannot give a trustworthy answer."""


def parse_duration(text):
    text = text.strip()
    if text == "0":
        return timedelta(0)
    match = re.fullmatch(r"(\d+)\s*([a-z]+?)s?", text)
    if not match or match.group(2) not in UNIT_SECONDS:
        raise CheckError(f"unrecognised duration {text!r}")
    return timedelta(seconds=int(match.group(1)) * UNIT_SECONDS[match.group(2)])


def read_min_age(config_path):
    try:
        config = tomllib.loads(Path(config_path).read_text())
    except FileNotFoundError:
        raise CheckError(f"{config_path} not found")
    value = config.get("registry", {}).get("global-min-publish-age")
    if value is None:
        raise CheckError(f"registry.global-min-publish-age is not set in {config_path}")
    return parse_duration(value)


def read_exceptions(path):
    try:
        data = tomllib.loads(Path(path).read_text())
    except FileNotFoundError:
        return {}
    exceptions = {}
    for entry in data.get("exception", []):
        missing = [k for k in ("crate", "version", "reason", "added") if not entry.get(k)]
        if missing:
            raise CheckError(f"exception {entry} in {path} is missing {', '.join(missing)}")
        exceptions[(entry["crate"], entry["version"])] = entry["reason"]
    return exceptions


def index_path(name):
    name = name.lower()
    if len(name) <= 2:
        return f"{len(name)}/{name}"
    if len(name) == 3:
        return f"3/{name[0]}/{name}"
    return f"{name[:2]}/{name[2:4]}/{name}"


def fetch_pubtime(index_url, name, version):
    url = f"{index_url.rstrip('/')}/{index_path(name)}"
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            body = response.read().decode()
    except (urllib.error.URLError, OSError) as e:
        raise CheckError(f"could not fetch index entry for {name}: {e}")
    for line in body.splitlines():
        if not line.strip():
            continue
        entry = json.loads(line)
        if entry.get("vers") == version:
            pubtime = entry.get("pubtime")
            if not pubtime:
                raise CheckError(f"index has no pubtime for {name} {version}")
            return datetime.fromisoformat(pubtime.replace("Z", "+00:00"))
    raise CheckError(f"{name} {version} not found in the index")


def load_cache(path):
    try:
        return json.loads(Path(path).read_text())
    except (FileNotFoundError, json.JSONDecodeError):
        return {}


def save_cache(path, cache):
    Path(path).write_text(json.dumps(cache, indent=0, sort_keys=True))


def check(lock_text, min_age, exceptions, cache, index_url, now):
    """Return (problems, notes); `cache` is updated with versions that passed."""
    lock = tomllib.loads(lock_text)
    problems, notes, to_fetch = [], [], []

    for package in lock.get("package", []):
        name, version, source = package["name"], package["version"], package.get("source")
        if source is None:
            continue
        if source not in CRATES_IO_SOURCES:
            problems.append(f"{name} {version}: comes from {source}; only crates.io is allowed")
            continue
        if f"{name}@{version}" not in cache:
            to_fetch.append((name, version))

    with ThreadPoolExecutor(max_workers=8) as pool:
        pubtimes = pool.map(lambda nv: fetch_pubtime(index_url, *nv), to_fetch)
        for (name, version), pubtime in zip(to_fetch, pubtimes):
            age = now - pubtime
            excepted = (name, version) in exceptions
            if age >= min_age:
                cache[f"{name}@{version}"] = pubtime.isoformat()
                if excepted:
                    notes.append(f"{name} {version} is now old enough; remove its exception")
            elif excepted:
                notes.append(f"{name} {version}: allowed by exception ({exceptions[(name, version)]})")
            else:
                problems.append(
                    f"{name} {version}: published {pubtime:%Y-%m-%d %H:%M} UTC, "
                    f"{age.days}d {age.seconds // 3600}h ago (minimum {min_age.days}d)"
                )

    for name, version in exceptions:
        if f"{name}@{version}" in cache and (name, version) not in to_fetch:
            notes.append(f"{name} {version} is now old enough; remove its exception")

    return problems, notes


def staged_lockfile():
    result = subprocess.run(
        ["git", "show", ":Cargo.lock"], capture_output=True, text=True, check=False
    )
    if result.returncode != 0:
        raise CheckError(f"could not read staged Cargo.lock: {result.stderr.strip()}")
    return result.stdout


def default_cache_path():
    git_dir = subprocess.run(
        ["git", "rev-parse", "--git-dir"], capture_output=True, text=True, check=True
    ).stdout.strip()
    return str(Path(git_dir) / "cooldown-cache.json")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--lockfile", help="check this file instead of the staged Cargo.lock")
    parser.add_argument("--config", default=".cargo/config.toml")
    parser.add_argument("--exceptions", default="cooldown-exceptions.toml")
    parser.add_argument("--cache", help="defaults to .git/cooldown-cache.json")
    parser.add_argument("--index-url", default=DEFAULT_INDEX_URL)
    parser.add_argument("--now", help="ISO timestamp to treat as now (for tests)")
    args = parser.parse_args(argv)

    try:
        min_age = read_min_age(args.config)
        exceptions = read_exceptions(args.exceptions)
        lock_text = Path(args.lockfile).read_text() if args.lockfile else staged_lockfile()
        cache_path = args.cache or default_cache_path()
        cache = load_cache(cache_path)
        now = datetime.fromisoformat(args.now) if args.now else datetime.now(timezone.utc)
        problems, notes = check(lock_text, min_age, exceptions, cache, args.index_url, now)
    except CheckError as e:
        print(f"cooldown check: {e}", file=sys.stderr)
        print("cooldown check: blocking commit because the check could not complete", file=sys.stderr)
        return 2

    save_cache(cache_path, cache)
    for note in notes:
        print(f"cooldown check: note: {note}", file=sys.stderr)
    if problems:
        print("cooldown check: these Cargo.lock entries are not allowed:", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        print(
            "Downgrade with `cargo update -p <crate> --precise <older version>`, or, if it is "
            "truly needed, add an entry to cooldown-exceptions.toml with a reason.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
