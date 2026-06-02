use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=GARYX_EMBED_WORKFLOW_BUN_XZ");
    println!("cargo:rerun-if-env-changed=GARYX_WORKFLOW_BUN_VERSION");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let output = out_dir.join("embedded_workflow_bun.rs");
    let version = env::var("GARYX_WORKFLOW_BUN_VERSION").unwrap_or_else(|_| "dev".to_owned());
    let target = env::var("TARGET").unwrap_or_else(|_| "unknown".to_owned());

    let Some(path) = env::var_os("GARYX_EMBED_WORKFLOW_BUN_XZ") else {
        fs::write(
            output,
            format!(
                "pub(super) const EMBEDDED_WORKFLOW_BUN_XZ: Option<&'static [u8]> = None;\n\
                 pub(super) const EMBEDDED_WORKFLOW_BUN_VERSION: &str = {version:?};\n\
                 pub(super) const EMBEDDED_WORKFLOW_BUN_TARGET: &str = {target:?};\n"
            ),
        )
        .expect("write embedded workflow bun metadata");
        return;
    };

    let path = PathBuf::from(path);
    let path = if path.is_absolute() {
        path
    } else {
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(path)
    };
    let path = path
        .canonicalize()
        .expect("GARYX_EMBED_WORKFLOW_BUN_XZ must point to a readable file");
    println!("cargo:rerun-if-changed={}", path.display());
    fs::write(
        output,
        format!(
            "pub(super) const EMBEDDED_WORKFLOW_BUN_XZ: Option<&'static [u8]> = Some(include_bytes!(r#\"{}\"#));\n\
             pub(super) const EMBEDDED_WORKFLOW_BUN_VERSION: &str = {version:?};\n\
             pub(super) const EMBEDDED_WORKFLOW_BUN_TARGET: &str = {target:?};\n",
            path.display()
        ),
    )
    .expect("write embedded workflow bun metadata");
}
