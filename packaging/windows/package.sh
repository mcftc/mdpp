#!/bin/sh
# Builds the Windows release files on Linux (or in Git Bash on Windows):
#   target/package/MD++-<version>-setup.exe              — installer (NSIS)
#   target/package/mdpp-<version>-x86_64-windows-portable.zip — unzip and run; settings stay beside it
# Needs: the x86_64-pc-windows-gnu Rust target, mingw-w64-gcc, vkd3d (shaders, see vendor/README.md),
# nsis (makensis) and bsdtar (libarchive).
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
target=x86_64-pc-windows-gnu
cargo build --locked --profile dist --target "$target" -p mdpp-app
stage="target/package/mdpp-$version-x86_64-windows"
rm -rf "$stage"
mkdir -p "$stage"
cp "target/$target/dist/mdpp.exe" README.md LICENSE-MIT LICENSE-APACHE "$stage/"
setup="target/package/MD++-$version-setup.exe"
makensis -V2 -DVERSION="$version" -DSOURCE="$root/$stage" -DOUTFILE="$root/$setup" packaging/windows/installer.nsi
portable="target/package/mdpp-$version-x86_64-windows-portable.zip"
rm -f "$portable"
touch "$stage/mdpp-portable" # portable mode: settings and session in mdpp-data\ beside mdpp.exe
(cd target/package && bsdtar -a -cf "$(basename "$portable")" "$(basename "$stage")") # bsdtar: also Windows 10+ tar.exe
rm "$stage/mdpp-portable"
for file in "$setup" "$portable"; do
    (cd target/package && sha256sum "$(basename "$file")" > "$(basename "$file").sha256")
    echo "$file"
done
