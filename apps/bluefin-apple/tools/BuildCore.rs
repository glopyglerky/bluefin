//! Build Apple core candidates without embedding the operator's source/home paths.
//! Run from apps/bluefin-apple. Generated output must live outside the repository.
use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn output(name: &str, repo: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let path = PathBuf::from(
        env::var_os(name).ok_or_else(|| format!("Set {name} to bounded build storage"))?,
    );
    if !path.is_absolute() || path.starts_with(repo) {
        return Err(format!("{name} must be absolute and outside source").into());
    }
    fs::create_dir_all(&path)?;
    let canonical = path.canonicalize()?;
    if canonical.starts_with(repo) {
        return Err(format!("{name} resolves inside source").into());
    }
    Ok(canonical)
}
fn captured(program: &str, args: &[&str]) -> Result<String, Box<dyn Error>> {
    let result = Command::new(program).args(args).output()?;
    if !result.status.success() {
        return Err(format!("{program} failed resolving the build environment").into());
    }
    Ok(String::from_utf8(result.stdout)?.trim().to_owned())
}
fn main() -> Result<(), Box<dyn Error>> {
    let app = env::current_dir()?.canonicalize()?;
    if !app.join("rust/Cargo.toml").is_file() {
        return Err("Run from apps/bluefin-apple".into());
    }
    let repo = app.ancestors().nth(2).ok_or("Missing repository")?;
    let target = output("CARGO_TARGET_DIR", repo)?;
    let destination = output("BLUEFIN_CORE_OUT", repo)?;
    let mut flags = env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default();
    if env::var_os("RUSTFLAGS").is_some() {
        return Err("Use CARGO_ENCODED_RUSTFLAGS for additional flags, not RUSTFLAGS".into());
    }
    let mut mappings = Vec::new();
    if let Some(home) = env::var_os("HOME") {
        mappings.push((PathBuf::from(home), "/build/user"));
    }
    if let Some(cargo) = env::var_os("CARGO_HOME") {
        mappings.push((PathBuf::from(cargo), "/cargo-home"));
    }
    mappings.push((
        PathBuf::from(captured("rustc", &["+stable", "--print", "sysroot"])?),
        "/rust-toolchain",
    ));
    mappings.push((repo.to_path_buf(), "/src/bluefin"));
    mappings.push((target.clone(), "/build/target"));
    for (path, replacement) in mappings {
        let path = path.to_str().ok_or("Build path must be UTF-8")?;
        if path.contains(['=', '\u{1f}']) {
            return Err("Build path cannot contain a mapping separator".into());
        }
        if !flags.is_empty() {
            flags.push('\u{1f}');
        }
        flags.push_str(&format!("--remap-path-prefix={path}={replacement}"));
    }
    let selection = env::args().nth(1);
    let targets = [
        ("aarch64-apple-ios-sim", "iphonesimulator"),
        ("aarch64-apple-ios", "iphoneos"),
        ("aarch64-apple-tvos", "appletvos"),
        ("aarch64-apple-tvos-sim", "appletvsimulator"),
    ];
    if env::args().len() > 2
        || selection
            .as_ref()
            .is_some_and(|value| !targets.iter().any(|(target, _)| target == value))
    {
        return Err("Supply one supported Apple target, or no argument for all four".into());
    }
    for (triple, sdk) in targets {
        if selection
            .as_ref()
            .is_some_and(|selected| selected != triple)
        {
            continue;
        }
        let sdk_root = captured("xcrun", &["--sdk", sdk, "--show-sdk-path"])?;
        println!("Building {triple} with source-path remapping");
        let status = Command::new("cargo")
            .args([
                "+stable",
                "build",
                "--locked",
                "--release",
                "--lib",
                "--target",
                triple,
            ])
            .current_dir(app.join("rust"))
            .env("SDKROOT", sdk_root)
            .env("CARGO_TARGET_DIR", &target)
            .env("CARGO_ENCODED_RUSTFLAGS", &flags)
            .env("IPHONEOS_DEPLOYMENT_TARGET", "15.0")
            .env("TVOS_DEPLOYMENT_TARGET", "15.0")
            .env("CARGO_BUILD_JOBS", "4")
            .status()?;
        if !status.success() {
            return Err(format!("Core build failed for {triple}").into());
        }
        let folder = destination.join(triple);
        fs::create_dir_all(&folder)?;
        fs::copy(
            target.join(triple).join("release/libbluefin_apple_core.a"),
            folder.join("libbluefin_apple_core.a"),
        )?;
        println!("Retained {triple} candidate");
    }
    Ok(())
}
