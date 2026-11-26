//! Dependency-free staging tool. Does not invent a vendor manifest or package.
use std::{env, fs, path::Path};
fn main() -> std::io::Result<()> {
    let a: Vec<_> = env::args().collect();
    assert_eq!(a.len(), 4, "stage <smarttv-source> <qualified-wasm> <new-output-dir>");
    let root = Path::new(&a[1]); let out = Path::new(&a[3]);
    fs::create_dir(out)?; // Refuse overwrite, including another worker's output.
    for name in ["index.html", "tv.css", "app.js", "policy.js", "icon.png"] {
        fs::copy(root.join("web").join(name), out.join(name))?;
    }
    fs::copy(root.join("vizio/platform.js"), out.join("platform.js"))?;
    fs::copy(&a[2], out.join("bluefin_tv_playback.wasm"))?;
    fs::write(out.join("PACKAGE-STATUS.txt"), "Hosted web candidate only. No Vizio partner package, signing, installation or firmware acceptance.\n")?;
    Ok(())
}
