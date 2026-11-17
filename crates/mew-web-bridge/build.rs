use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let dist = manifest.join("../../mew-web-ui/dist");
    println!("cargo:rerun-if-changed={}", dist.display());
    let assets = if dist.is_dir() {
        dist
    } else {
        let empty = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("empty-ui");
        std::fs::create_dir_all(&empty)?;
        empty
    };
    println!("cargo:rustc-env=MEW_WEB_UI_DIST={}", assets.display());
    Ok(())
}
