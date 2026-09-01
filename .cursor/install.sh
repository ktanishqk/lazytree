#!/usr/bin/env bash
# Idempotent Cloud Agent setup for LazyTree (a Rust CLI whose COW workspace
# sessions mount overlay filesystems). Safe to re-run: every step is a no-op
# once satisfied.
set -euo pipefail

# 1. System dependency: fuse-overlayfs.
#    LazyTree prefers kernel OverlayFS and falls back to fuse-overlayfs, which
#    is the backend that works unprivileged inside nested/Cloud Agent VMs.
#    Guarding with `command -v` keeps re-runs fast; the dpkg options avoid an
#    interactive /etc/fuse.conf conffile prompt on images that ship one.
if ! command -v fuse-overlayfs >/dev/null 2>&1; then
  sudo DEBIAN_FRONTEND=noninteractive apt-get update -qq
  sudo DEBIAN_FRONTEND=noninteractive apt-get install -y \
    -o Dpkg::Options::=--force-confold fuse-overlayfs
fi

# 2. Rust toolchain. The committed Cargo.lock pins crates that require the 2024
#    edition (Rust >= 1.85), which is newer than some base images default to,
#    so pin the build to the stable channel.
rustup toolchain install stable --profile minimal --no-self-update
rustup default stable

# 3. Build the release binary against the locked dependency set.
cargo build --release --locked
