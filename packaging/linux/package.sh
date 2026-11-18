#!/bin/sh
# Builds the Linux release archive: target/package/mdpp-<version>-<arch>-linux.tar.gz (+ .sha256).
# The archive holds the program, install.sh (menu entry, icon, .md association) and the docs.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
name="mdpp-$version-$(uname -m)-linux"
cargo build --locked --profile dist -p mdpp-app
stage="target/package/$name"
rm -rf "$stage" "$stage.tar.gz"
mkdir -p "$stage"
cp target/dist/mdpp "$stage/"
cp packaging/linux/io.github.mcftc.mdpp.* packaging/linux/install.sh "$stage/"
cp README.md LICENSE-MIT LICENSE-APACHE "$stage/"
tar -C target/package -czf "$stage.tar.gz" "$name"
(cd target/package && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
echo "target/package/$name.tar.gz"
