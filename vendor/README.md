# Vendored crates

## gpui-pre-windows 0.3.12

GPUI's Windows backend (Apache-2.0, `LICENSE-APACHE`), unchanged except for `build.rs`: a release
build for Windows compiles the HLSL shaders with `fxc.exe`, which only exists on Windows. The added
branch runs when building *for* Windows *on* another system and compiles the same shaders with
`vkd3d-compiler` (Wine's HLSL compiler, package `vkd3d`; another path via `VKD3D_COMPILER`).

Used through `[patch.crates-io]` in the workspace `Cargo.toml`. When gpui-kit moves to a newer
`gpui-pre`, copy the new crate here and apply the same `build.rs` addition (or drop the patch if
upstream gains a cross-build path).
