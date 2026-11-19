//! Windows builds get MD++'s icon and version information (`packaging/windows/mdpp.rc`, a template
//! filled here); nothing happens for other targets.

use std::path::PathBuf;

fn main() {
    let packaging = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packaging/windows");
    let (template, icon) = (packaging.join("mdpp.rc"), packaging.join("mdpp.ico"));
    println!("cargo:rerun-if-changed={}", template.display());
    println!("cargo:rerun-if-changed={}", icon.display());
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let version = env!("CARGO_PKG_VERSION");
    let number = |name: &str| std::env::var(name).unwrap_or_else(|_| "0".into());
    let icon = icon
        .canonicalize()
        .expect("packaging/windows/mdpp.ico exists");
    let filled = std::fs::read_to_string(&template)
        .expect("packaging/windows/mdpp.rc exists")
        .replace("@ICON@", &icon.display().to_string().replace('\\', "/"))
        .replace("@MAJOR@", &number("CARGO_PKG_VERSION_MAJOR"))
        .replace("@MINOR@", &number("CARGO_PKG_VERSION_MINOR"))
        .replace("@PATCH@", &number("CARGO_PKG_VERSION_PATCH"))
        .replace("@VERSION@", version);
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("mdpp.rc");
    std::fs::write(&out, filled).unwrap();
    embed_resource::compile(&out, embed_resource::NONE)
        .manifest_optional()
        .expect("Windows resources compile");
}
