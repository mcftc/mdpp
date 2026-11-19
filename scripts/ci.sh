#!/bin/sh
# The project's CI, run on this machine (no CI service): formatting, lints, tests, dependency
# policy, the "core crates stay UI-free" rule, and a Windows check build. The pre-push hook runs it
# (.githooks/pre-push; enable once with `git config core.hooksPath .githooks`).
#   scripts/ci.sh          everything
#   scripts/ci.sh --quick  skip the Windows check and cargo-deny
set -eu
cd "$(dirname "$0")/.."
step() { printf '\n== %s\n' "$1"; }

step "format"
cargo fmt --all -- --check
step "lints"
RUSTFLAGS="-D warnings" cargo clippy --quiet --workspace --all-targets
step "tests"
cargo test --quiet --workspace
step "core crates stay UI-free (PLAN §3.1)"
if cargo tree -p mdpp-core -p mdpp-markdown -p mdpp-lang -e normal --prefix none | grep -E '^gpui'; then
    echo "mdpp-core, mdpp-markdown and mdpp-lang must not depend on GPUI" >&2
    exit 1
fi
if [ "${1:-}" != "--quick" ]; then
    step "dependency policy"
    cargo deny check
    step "Windows build check"
    cargo check --quiet --target x86_64-pc-windows-gnu -p mdpp-app
fi
step "CI passed"
