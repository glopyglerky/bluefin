//! Build apps using exact-platform retained media archives.
// The separate host-only legal stager is built before Xcode; Rust media archives
// are always supplied and are never rebuilt by this helper.
#[path = "LegalResources.rs"]
mod legal_resources;
use std::{env, error::Error, fs, path::PathBuf, process::Command};
fn main() -> Result<(), Box<dyn Error>> {
    let app = env::current_dir()?.canonicalize()?;
    if !app.join("project.yml").is_file() {
        return Err("Run from apps/bluefin-apple".into());
    }
    let repo = app.ancestors().nth(2).ok_or("Missing repository")?;
    let selection = env::args()
        .nth(1)
        .ok_or("Select mac-arm64, mac-intel26, vision, vision-sim, watch or watch-sim")?;
    if env::args().len() != 2 {
        return Err("Supply exactly one destination".into());
    }
    let (scheme, sdk, platform, arch, triple, products) = match selection.as_str() {
        "mac-arm64" => (
            "BluefinMac",
            "macosx",
            "macOS,variant=Mac Catalyst",
            "arm64",
            Some("aarch64-apple-ios-macabi"),
            "Release-maccatalyst",
        ),
        "mac-intel26" => (
            "BluefinMac",
            "macosx",
            "macOS,variant=Mac Catalyst",
            "x86_64",
            Some("x86_64-apple-ios-macabi"),
            "Release-maccatalyst",
        ),
        "vision" => (
            "BluefinVision",
            "xros",
            "visionOS",
            "arm64",
            Some("aarch64-apple-visionos"),
            "Release-xros",
        ),
        "vision-sim" => (
            "BluefinVision",
            "xrsimulator",
            "visionOS Simulator",
            "arm64",
            Some("aarch64-apple-visionos-sim"),
            "Release-xrsimulator",
        ),
        "watch" => (
            "BluefinWatch",
            "watchos",
            "watchOS",
            "arm64 arm64_32",
            None,
            "Release-watchos",
        ),
        "watch-sim" => (
            "BluefinWatch",
            "watchsimulator",
            "watchOS Simulator",
            "arm64",
            None,
            "Release-watchsimulator",
        ),
        _ => return Err("Unknown destination".into()),
    };
    let mut paths = Vec::new();
    for name in ["BLUEFIN_XCODE_DERIVED", "BLUEFIN_APP_OUT"] {
        let p = PathBuf::from(env::var_os(name).ok_or(format!("Set {name}"))?);
        if !p.is_absolute() {
            return Err("Output paths must be absolute".into());
        }
        fs::create_dir_all(&p)?;
        let p = p.canonicalize()?;
        if p.starts_with(repo) {
            return Err("Output must stay outside source".into());
        }
        let p = p.join(&selection);
        if p.exists() {
            return Err("Use fresh output roots to preserve earlier candidates".into());
        }
        paths.push(p);
    }
    let mut c = Command::new("xcodebuild");
    let legal_tool =
        legal_resources::prepare(repo, paths[0].parent().ok_or("Missing build root")?)?;
    c.args([
        "-project",
        "Bluefin.xcodeproj",
        "-scheme",
        scheme,
        "-configuration",
        "Release",
        "-sdk",
        sdk,
        "-destination",
    ])
    .arg(format!("generic/platform={platform}"))
    .args(["-jobs", "1", "-derivedDataPath"])
    .arg(&paths[0])
    .arg("CODE_SIGNING_ALLOWED=NO")
    .arg(if selection == "watch" {
        "ONLY_ACTIVE_ARCH=NO"
    } else {
        "ONLY_ACTIVE_ARCH=YES"
    })
    .arg(format!("ARCHS={arch}"));
    c.arg(format!(
        "BLUEFIN_LEGAL_RESOURCES_TOOL={}",
        legal_tool.tool.display()
    ));
    c.arg(format!(
        "BLUEFIN_LEGAL_FILELIST_DIR={}",
        legal_tool.lists.display()
    ));
    if let Some(triple) = triple {
        let core = PathBuf::from(env::var_os("BLUEFIN_CORE_OUT").ok_or("Set BLUEFIN_CORE_OUT")?)
            .canonicalize()?
            .join(triple);
        if !core.join("libbluefin_apple_core.a").is_file() {
            return Err(format!("Missing {triple}; iOS objects cannot substitute").into());
        }
        c.arg(format!("BLUEFIN_RUST_LIB_DIR={}", core.display()));
    } else {
        c.args([
            "OTHER_LDFLAGS=",
            "LIBRARY_SEARCH_PATHS=",
            "SWIFT_OBJC_BRIDGING_HEADER=",
        ]);
    }
    if !c.arg("build").status()?.success() {
        return Err("Platform build failed; retain first failure".into());
    }
    let bundle = format!("{scheme}.app");
    let source = paths[0].join("Build/Products").join(products).join(&bundle);
    if !source.is_dir() {
        return Err("Expected bundle missing".into());
    }
    fs::create_dir(&paths[1])?;
    if !Command::new("ditto")
        .arg(source)
        .arg(paths[1].join(bundle))
        .status()?
        .success()
    {
        return Err("Bundle retention failed".into());
    }
    println!("Retained unsigned {selection}; runtime and signing gates remain");
    Ok(())
}
