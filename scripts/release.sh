#!/bin/sh
# Makes a release on this machine (no CI service): runs the CI, builds the Linux archive, the
# Windows setup and the Windows portable zip, then — with --publish — tags v<version> and uploads
# everything to GitHub Releases with the gh CLI. The version is the one in Cargo.toml.
#   scripts/release.sh            build and check only (files in target/package/)
#   scripts/release.sh --publish  also tag, push the tag and publish (as a pre-release while < 1.0)
set -eu
cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
tag="v$version"

scripts/ci.sh
packaging/linux/package.sh
packaging/windows/package.sh

files=$(ls target/package/*"$version"*.tar.gz* target/package/*"$version"*setup.exe* target/package/*"$version"*portable.zip*)
printf '\nRelease files:\n%s\n' "$files"
[ "${1:-}" = "--publish" ] || exit 0

if [ -n "$(git status --porcelain)" ]; then
    echo "Commit or stash the changes first: a release is made from a clean tree." >&2
    exit 1
fi
git rev-parse -q --verify "refs/tags/$tag" >/dev/null || git tag -a "$tag" -m "MD++ $version"
git push origin "$tag"
prerelease=""
case "$version" in 0.*) prerelease="--prerelease" ;; esac
if gh release view "$tag" >/dev/null 2>&1; then
    # shellcheck disable=SC2086
    gh release upload "$tag" $files --clobber
else
    # shellcheck disable=SC2086
    gh release create "$tag" $files --title "MD++ $version" --generate-notes $prerelease
fi
gh release view "$tag" --json url -q .url
