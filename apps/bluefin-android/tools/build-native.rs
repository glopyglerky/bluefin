//! Run with rustc on a Linux Android build host, then invoke with source root,
//! NDK root, bounded target directory, and final JNI output directory.
use std::{env, fs, path::PathBuf, process::Command};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args_os().skip(1).map(PathBuf::from).collect();
    if args.len() != 4 {
        return Err("expected SOURCE_ROOT NDK_ROOT TARGET_DIR JNI_DIR".into());
    }
    let root = args[0].canonicalize()?;
    let ndk = args[1].canonicalize()?;
    let bin = ndk.join("toolchains/llvm/prebuilt/linux-x86_64/bin");
    let manifest = root.join("apps/bluefin-android/rust/Cargo.toml");
    let cargo_home = PathBuf::from(env::var_os("CARGO_HOME").ok_or("set CARGO_HOME")?);
    if !bin.join("llvm-ar").is_file() {
        return Err("Linux x86_64 NDK required".into());
    }
    let boundary = Command::new("cargo")
        .args(["run", "--locked", "--manifest-path"])
        .arg(root.join("gates/client-boundary/Cargo.toml"))
        .arg("--")
        .arg(&root)
        .arg("android")
        .env("CARGO_TARGET_DIR", args[2].join("client-boundary"))
        .status()?;
    if !boundary.success() {
        return Err("Android client dependency boundary failed".into());
    }
    let retention = Command::new("cargo")
        .args(["run", "--locked", "--manifest-path"])
        .arg(root.join("apps/bluefin-apple/iroh-modern/qa/compaction-retention/Cargo.toml"))
        .env("CARGO_TARGET_DIR", args[2].join("compaction-retention"))
        .status()?;
    if !retention.success() {
        return Err("Selected Noq compaction retained-allocation regression failed".into());
    }
    for (target, clang, abi) in [
        (
            "aarch64-linux-android",
            "aarch64-linux-android26-clang",
            "arm64-v8a",
        ),
        (
            "armv7-linux-androideabi",
            "armv7a-linux-androideabi26-clang",
            "armeabi-v7a",
        ),
        (
            "x86_64-linux-android",
            "x86_64-linux-android26-clang",
            "x86_64",
        ),
    ] {
        let linker = bin.join(clang);
        let key = target.replace('-', "_");
        let rustflags = format!(
            "--remap-path-prefix={}=/bluefin --remap-path-prefix={}=/cargo -C link-arg=-Wl,-z,max-page-size=16384 -C strip=debuginfo",
            root.display(),
            cargo_home.display()
        );
        let status = Command::new("cargo")
            .args([
                "build",
                "--locked",
                "--release",
                "-j",
                "2",
                "--target",
                target,
                "--manifest-path",
            ])
            .arg(&manifest)
            .env("CARGO_TARGET_DIR", &args[2])
            .env(
                format!("CARGO_TARGET_{}_LINKER", key.to_uppercase()),
                &linker,
            )
            .env(format!("CC_{key}"), &linker)
            .env(format!("AR_{key}"), bin.join("llvm-ar"))
            .env(
                format!("CFLAGS_{key}"),
                format!(
                    "-ffile-prefix-map={}=/bluefin -ffile-prefix-map={}=/cargo",
                    root.display(),
                    cargo_home.display()
                ),
            )
            .env("RUSTFLAGS", &rustflags)
            .status()?;
        if !status.success() {
            return Err(format!("native build failed for {target}: {status}").into());
        }
        let out = args[3].join(abi);
        fs::create_dir_all(&out)?;
        fs::copy(
            args[2].join(target).join("release/libbluefin_android.so"),
            out.join("libbluefin_android.so"),
        )?;
    }
    Ok(())
}
