#!/bin/bash
# Build universal (fat) binaries for tunnelclient and tunnel-bootstrap.
# Combines x86_64 (Intel) and aarch64 (Apple Silicon) slices via lipo.
set -e

export MACOSX_DEPLOYMENT_TARGET=10.13

cargo build --release --target x86_64-apple-darwin -p tunnelclient
cargo build --release --target aarch64-apple-darwin -p tunnelclient

cargo build --release --target x86_64-apple-darwin -p tunnel-bootstrap
cargo build --release --target aarch64-apple-darwin -p tunnel-bootstrap

mkdir -p dist

lipo -create \
  target/x86_64-apple-darwin/release/tunnelclient \
  target/aarch64-apple-darwin/release/tunnelclient \
  -output dist/tunnelclient

lipo -create \
  target/x86_64-apple-darwin/release/tunnel-bootstrap \
  target/aarch64-apple-darwin/release/tunnel-bootstrap \
  -output dist/tunnel-bootstrap

echo "Universal binaries written to dist/"
