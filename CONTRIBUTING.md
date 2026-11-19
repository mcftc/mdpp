# Contributing to MD++

Thank you for helping! MD++ is free and open source (MIT OR Apache-2.0).

## Build and run

```sh
cargo run -p mdpp-app -- path/to/file.md      # debug build
cargo build --release -p mdpp-app             # optimized build → target/release/mdpp
cargo test --workspace
```

Linux needs the GUI development packages (Debian/Ubuntu: `libxkbcommon-dev libxkbcommon-x11-dev
libwayland-dev libfontconfig-dev libfreetype-dev libvulkan-dev libx11-xcb-dev`).
The toolchain version is pinned in `rust-toolchain.toml`; `rustup` picks it up automatically.

## Where things live

| Crate | What | Depends on a UI toolkit? |
|---|---|---|
| `mdpp-core` | documents, files, encodings, settings | no |
| `mdpp-markdown` | highlighting lexer, document structure | no |
| `mdpp-lang` | completions, diagnostics | no |
| `mdpp-app` | the GPUI / gpui-kit desktop app | yes |

Most logic and most tests belong in the first three; `scripts/ci.sh` fails if they gain a GPUI
dependency.
Design decisions and their evidence are in [docs/PLAN.md](docs/PLAN.md) and
[docs/RESEARCH.md](docs/RESEARCH.md) — please read the relevant section before larger changes.

## CI and releases

There is no CI service: the checks run on the developer's machine.

- `scripts/ci.sh` — formatting, lints (warnings fail), tests, `cargo deny`, the UI-free rule and a
  Windows check build; `--quick` skips the last two. Enable it before every push once with
  `git config core.hooksPath .githooks`.
- `scripts/release.sh` — CI, then the Linux archive (`packaging/linux/package.sh`), the Windows
  setup and portable zip (`packaging/windows/package.sh`, cross-built on Linux: needs the
  `x86_64-pc-windows-gnu` Rust target, `mingw-w64-gcc`, `vkd3d` and `nsis`); `--publish` also tags
  `v<version>` and uploads to GitHub Releases with `gh`.

## Before you open a pull request

- `scripts/ci.sh` passes (or at least `cargo fmt --all`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`).
- New behaviour comes with tests. Performance-sensitive code: run
  `cargo test --release -p mdpp-lang -- --ignored --nocapture` and, for the app, the built-in
  measurements (`MDPP_BENCH=startup|typing:300|idle:15 target/release/mdpp file.md`).
- New dependencies: say in the PR why an existing crate or a GPUI API could not do the job, and
  check `cargo deny check`.
- Tunable values go into settings or data files, not into code.
- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org) (`feat:`, `fix:`, …);
  the changelog is generated from them.
