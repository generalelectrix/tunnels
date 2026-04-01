#!/bin/bash
# Build universal (fat) binaries for all tunnels executables.
# Combines x86_64 (Intel) and aarch64 (Apple Silicon) slices via lipo.
set -e

export MACOSX_DEPLOYMENT_TARGET=10.13

PACKAGES="-p console -p tunnelclient -p tunnel-bootstrap -p bootstrap-deploy"

cargo build --release --target x86_64-apple-darwin $PACKAGES
cargo build --release --target aarch64-apple-darwin $PACKAGES

mkdir -p dist

for bin in console tunnelclient tunnel-bootstrap bootstrap-deploy; do
  lipo -create \
    "target/x86_64-apple-darwin/release/$bin" \
    "target/aarch64-apple-darwin/release/$bin" \
    -output "dist/$bin"
done

echo "Universal binaries written to dist/"
