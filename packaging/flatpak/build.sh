#!/usr/bin/env bash
# Swiftify, a spotify player for desktop. Copyright (C) 2026 shred0s, GPL-3.0-or-later
# builds the flatpak, installs it for this user, and writes a bundle beside it:
#   packaging/flatpak/build.sh            build and install
#   packaging/flatpak/build.sh --bundle   also write swiftify.flatpak
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
manifest="$here/dev.shredos.Swiftify.yml"
cd "$root"

command -v flatpak-builder >/dev/null || { echo "flatpak-builder is not installed" >&2; exit 1; }
flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo

# the crates follow Cargo.lock: regenerate when it is newer
if [[ spotify-core/Cargo.lock -nt "$here/cargo-sources.json" ]]; then
    python3 -I "$here/cargo-sources.py" spotify-core/Cargo.lock > "$here/cargo-sources.json"
fi

flatpak-builder --user --install-deps-from=flathub --install --force-clean \
    --repo=flatpak-repo build/flatpak "$manifest"

if [[ "${1:-}" == "--bundle" ]]; then
    flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
        flatpak-repo swiftify.flatpak dev.shredos.Swiftify
    echo "wrote $root/swiftify.flatpak"
fi
echo "run it with: flatpak run dev.shredos.Swiftify"
