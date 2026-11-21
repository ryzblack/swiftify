#!/usr/bin/env python3
# Swiftify, a spotify player for desktop
# Copyright (C) 2026  shred0s
# SPDX-License-Identifier: GPL-3.0-or-later
"""Write the flatpak-builder sources that let cargo build spotify-core offline.

Every crate in Cargo.lock becomes an archive from static.crates.io, checked
against the lock's own sha256 and unpacked into cargo/vendor, plus the cargo
config that points cargo at it. Only crates.io packages are supported (the
lock has no git ones); anything else stops the script instead of being skipped.

    packaging/flatpak/cargo-sources.py spotify-core/Cargo.lock > packaging/flatpak/cargo-sources.json
    packaging/flatpak/cargo-sources.py --self-test
"""
import json
import sys
import tomllib

REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"


def sources(lock):
    result = []
    for package in lock.get("package", []):
        source = package.get("source")
        if source is None:
            continue  # a path dependency, in the tree already
        if source != REGISTRY:
            raise SystemExit(f"unsupported source for {package['name']}: {source}")
        name, version, checksum = package["name"], package["version"], package["checksum"]
        directory = f"cargo/vendor/{name}-{version}"
        result.append({
            "type": "archive",
            "archive-type": "tar-gzip",
            "url": f"https://static.crates.io/crates/{name}/{name}-{version}.crate",
            "sha256": checksum,
            "dest": directory,
        })
        result.append({
            "type": "inline",
            "contents": json.dumps({"package": checksum, "files": {}}),
            "dest": directory,
            "dest-filename": ".cargo-checksum.json",
        })
    result.append({
        "type": "inline",
        "contents": '[source.vendored-sources]\ndirectory = "cargo/vendor"\n\n'
                    '[source.crates-io]\nreplace-with = "vendored-sources"\n',
        "dest": "cargo",
        "dest-filename": "config.toml",
    })
    return result


def self_test():
    lock = tomllib.loads(
        '[[package]]\nname = "a"\nversion = "1.0.0"\nsource = "%s"\nchecksum = "ff"\n'
        '[[package]]\nname = "local"\nversion = "0.1.0"\n' % REGISTRY)
    out = sources(lock)
    assert out[0]["url"] == "https://static.crates.io/crates/a/a-1.0.0.crate"
    assert out[0]["sha256"] == "ff" and out[0]["dest"] == "cargo/vendor/a-1.0.0"
    assert json.loads(out[1]["contents"]) == {"package": "ff", "files": {}}
    assert len(out) == 3, "the path dependency is left out"
    try:
        sources(tomllib.loads('[[package]]\nname = "g"\nversion = "1"\nsource = "git+https://x"\n'))
    except SystemExit:
        pass
    else:
        raise AssertionError("a git source must stop the script")
    print("ok")


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        self_test()
    else:
        with open(sys.argv[1], "rb") as handle:
            json.dump(sources(tomllib.load(handle)), sys.stdout, indent=2)
            sys.stdout.write("\n")
