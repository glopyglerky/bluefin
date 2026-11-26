//! Build the host-only notice stager before Xcode enters its script sandbox.
use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub struct Resources {
    pub tool: PathBuf,
    pub lists: PathBuf,
}

pub fn prepare(repo: &Path, derived: &Path) -> Result<Resources, Box<dyn Error>> {
    let host = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        _ => return Err("Apple app packaging requires a native Mac build host".into()),
    };
    let target = derived.join("legal-host-target");
    fs::create_dir_all(&target)?;
    let target = target.canonicalize()?;
    if target.starts_with(repo) {
        return Err("Host build output must stay outside source".into());
    }
    if !Command::new("cargo")
        .args([
            "build",
            "--release",
            "--locked",
            "--target",
            host,
            "--manifest-path",
        ])
        .arg(repo.join("gates/apple-legal-resources/Cargo.toml"))
        .env("CARGO_TARGET_DIR", &target)
        .status()?
        .success()
    {
        return Err("The local legal-resource stager could not be built".into());
    }
    let tool = target.join(host).join("release/bluefin-legal-resources");
    if !tool.is_file() {
        return Err("The local legal-resource stager is missing".into());
    }
    let lists = derived.join("legal-filelists");
    if !Command::new(&tool)
        .arg("--xcode-filelists")
        .arg(repo.join("docs/legal/bluefin-clients"))
        .arg(&lists)
        .status()?
        .success()
    {
        return Err("Target-specific legal resource file lists could not be prepared".into());
    }
    Ok(Resources { tool, lists })
}
