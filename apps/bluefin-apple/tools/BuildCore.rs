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
// cc parses these values with Shlex when CC_SHELL_ESCAPED_FLAGS is enabled.
// Quote individual arguments, including spaces, apostrophes and shell metacharacters.
fn quote_flag(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
fn native_environment() -> Result<Vec<(String, String)>, Box<dyn Error>> {
    let escaped = env::var("CC_SHELL_ESCAPED_FLAGS")
        .is_ok_and(|v| v != "0" && v != "false" && v != "no" && !v.is_empty());
    let mut flags = Vec::new();
    for (key, value) in env::vars_os() {
        let Some(key) = key.to_str() else {
            continue;
        };
        if ["CFLAGS", "CXXFLAGS", "ARFLAGS"].iter().any(|kind| {
            key == *kind
                || key.starts_with(&format!("{kind}_"))
                || key == format!("TARGET_{kind}")
                || key == format!("HOST_{kind}")
        }) {
            let value = value.to_str().ok_or("Native flags must be UTF-8")?;
            flags.push((
                key.to_owned(),
                if escaped {
                    value.to_owned()
                } else {
                    value
                        .split_ascii_whitespace()
                        .map(quote_flag)
                        .collect::<Vec<_>>()
                        .join(" ")
                },
            ));
        }
    }
    Ok(flags)
}
fn main() -> Result<(), Box<dyn Error>> {
    let app = env::current_dir()?.canonicalize()?;
    if !app.join("rust/Cargo.toml").is_file() {
        return Err("Run from apps/bluefin-apple".into());
    }
    let repo = app.ancestors().nth(2).ok_or("Missing repository")?;
    let jobs = env::var("CARGO_BUILD_JOBS")
        .unwrap_or_else(|_| "2".into())
        .parse::<usize>()?;
    if !(1..=4).contains(&jobs) {
        return Err("CARGO_BUILD_JOBS must be between 1 and 4".into());
    }
    let jobs = jobs.to_string();
    let toolchain = format!(
        "+{}",
        env::var("BLUEFIN_RUST_TOOLCHAIN").unwrap_or_else(|_| "stable".into())
    );
    let target = output("CARGO_TARGET_DIR", repo)?;
    let destination = output("BLUEFIN_CORE_OUT", repo)?;
    let boundary = Command::new("cargo")
        .args([&toolchain, "run", "--locked", "--manifest-path"])
        .arg(repo.join("gates/client-boundary/Cargo.toml"))
        .arg("--")
        .arg(repo)
        .env("CARGO_TARGET_DIR", target.join("client-boundary"))
        .env("CARGO_BUILD_JOBS", &jobs)
        .status()?;
    if !boundary.success() {
        return Err("Native client dependency boundary failed".into());
    }
    let retention = Command::new("cargo")
        .args([&toolchain, "run", "--locked", "--manifest-path"])
        .arg(app.join("iroh-modern/qa/compaction-retention/Cargo.toml"))
        .env("CARGO_TARGET_DIR", target.join("compaction-retention"))
        .env("CARGO_BUILD_JOBS", &jobs)
        .status()?;
    if !retention.success() {
        return Err("Selected Noq compaction retained-allocation regression failed".into());
    }
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
        PathBuf::from(captured("rustc", &[&toolchain, "--print", "sysroot"])?),
        "/rust-toolchain",
    ));
    mappings.push((repo.to_path_buf(), "/src/bluefin"));
    mappings.push((target.clone(), "/build/target"));
    let inherited_native = native_environment()?;
    let mut native_flags = String::from("-fdebug-compilation-dir=/build");
    for (path, replacement) in mappings {
        let path = path.to_str().ok_or("Build path must be UTF-8")?;
        if path.contains(['=', '\u{1f}']) {
            return Err("Build path cannot contain a mapping separator".into());
        }
        if !flags.is_empty() {
            flags.push('\u{1f}');
        }
        flags.push_str(&format!("--remap-path-prefix={path}={replacement}"));
        for kind in ["file", "debug", "macro"] {
            native_flags.push(' ');
            native_flags.push_str(&quote_flag(&format!(
                "-f{kind}-prefix-map={path}={replacement}"
            )));
        }
    }
    let selection = env::args().nth(1);
    let targets = [
        ("aarch64-apple-ios-sim", "iphonesimulator"),
        ("aarch64-apple-ios", "iphoneos"),
        ("aarch64-apple-tvos", "appletvos"),
        ("aarch64-apple-tvos-sim", "appletvsimulator"),
        ("aarch64-apple-ios-macabi", "macosx"),
        ("x86_64-apple-ios-macabi", "macosx"),
        ("aarch64-apple-visionos", "xros"),
        ("aarch64-apple-visionos-sim", "xrsimulator"),
    ];
    if env::args().len() > 2
        || selection
            .as_ref()
            .is_some_and(|value| !targets.iter().any(|(target, _)| target == value))
    {
        return Err("Supply one supported Apple target, or no argument for iOS and tvOS".into());
    }
    let sdk_override = env::var("BLUEFIN_APPLE_SDK").ok();
    if sdk_override.is_some() && selection.is_none() {
        return Err("An explicit SDK requires selecting one target".into());
    }
    for (index, (triple, sdk)) in targets.into_iter().enumerate() {
        // Keep the existing default build bounded to its four iOS/tvOS outputs.
        if selection.is_none() && index >= 4 {
            continue;
        }
        if selection
            .as_ref()
            .is_some_and(|selected| selected != triple)
        {
            continue;
        }
        let sdk_root = captured(
            "xcrun",
            &[
                "--sdk",
                sdk_override.as_deref().unwrap_or(sdk),
                "--show-sdk-path",
            ],
        )?;
        let floor = if index >= 4 { "26.0" } else { "15.0" };
        println!("Building {triple} with source-path remapping");
        let mut native = inherited_native.clone();
        for kind in ["CFLAGS", "CXXFLAGS"] {
            let old = native
                .iter()
                .find(|(key, _)| key == kind)
                .map(|(_, value)| value.as_str())
                .unwrap_or("");
            let value = format!(
                "{old} {native_flags} {}",
                quote_flag(&format!("-ffile-prefix-map={sdk_root}=/apple-sdk"))
            );
            native.retain(|(key, _)| key != kind);
            native.push((kind.into(), value));
        }
        let status = Command::new("cargo")
            .args([
                &toolchain,
                "build",
                "--locked",
                "--release",
                "--lib",
                "--target",
                triple,
            ])
            .current_dir(app.join("rust"))
            .envs(native)
            .env("CC_SHELL_ESCAPED_FLAGS", "1")
            .env("SDKROOT", sdk_root)
            .env("CARGO_TARGET_DIR", &target)
            .env("CARGO_ENCODED_RUSTFLAGS", &flags)
            .env("IPHONEOS_DEPLOYMENT_TARGET", floor)
            .env("TVOS_DEPLOYMENT_TARGET", floor)
            .env("XROS_DEPLOYMENT_TARGET", floor)
            .env("MACOSX_DEPLOYMENT_TARGET", "26.0")
            .env("CARGO_BUILD_JOBS", &jobs)
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
