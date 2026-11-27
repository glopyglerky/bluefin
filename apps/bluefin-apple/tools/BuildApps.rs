//! Build unsigned Apple candidates. Run from apps/bluefin-apple after BuildCore.rs.
#[path = "LegalResources.rs"]
mod legal_resources;
use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn outside(name: &str, source: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let path = PathBuf::from(env::var_os(name).ok_or_else(|| format!("Set {name}"))?);
    if !path.is_absolute() || path.starts_with(source) {
        return Err(format!("{name} must be absolute and outside source").into());
    }
    fs::create_dir_all(&path)?;
    let path = path.canonicalize()?;
    if path.starts_with(source) {
        return Err(format!("{name} resolves inside source").into());
    }
    Ok(path)
}
fn copy_tree(source: &Path, destination: &Path) -> Result<(), Box<dyn Error>> {
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Invalid bundle member name")?;
    if name.ends_with(".xctest") || name.ends_with(".xctest.dSYM") || name == "ServerModelFixture" {
        return Err(
            "Test payload found in application output; build with a fresh derived-data directory"
                .into(),
        );
    }
    // A fresh destination preserves earlier candidates and prevents accidental replacement.
    fs::create_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let to = destination.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &to)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), to)?;
        } else {
            return Err("Unexpected non-file in application bundle".into());
        }
    }
    Ok(())
}
fn main() -> Result<(), Box<dyn Error>> {
    let app = env::current_dir()?.canonicalize()?;
    let project = match env::var("BLUEFIN_APP_PROFILE") {
        Err(env::VarError::NotPresent) => "Bluefin.xcodeproj",
        Ok(profile) if profile == "provisioned" => "Bluefin.xcodeproj",
        Ok(profile) if profile == "personal" => "BluefinPersonal.xcodeproj",
        _ => return Err("BLUEFIN_APP_PROFILE must be personal or provisioned".into()),
    };
    if !app.join(project).join("project.pbxproj").is_file() {
        return Err(format!("Generate {project} in apps/bluefin-apple first").into());
    }
    let repo = app.ancestors().nth(2).ok_or("Missing repository")?;
    let derived = outside("BLUEFIN_XCODE_DERIVED", repo)?;
    let output = outside("BLUEFIN_APP_OUT", repo)?;
    let core = PathBuf::from(env::var_os("BLUEFIN_CORE_OUT").ok_or("Set BLUEFIN_CORE_OUT")?)
        .canonicalize()?;
    let targets = [
        (
            "ios-sim",
            "Bluefin",
            "aarch64-apple-ios-sim",
            "iOS Simulator",
            "Release-iphonesimulator",
        ),
        (
            "ios",
            "Bluefin",
            "aarch64-apple-ios",
            "iOS",
            "Release-iphoneos",
        ),
        (
            "tvos-sim",
            "BluefinTV",
            "aarch64-apple-tvos-sim",
            "tvOS Simulator",
            "Release-appletvsimulator",
        ),
        (
            "tvos",
            "BluefinTV",
            "aarch64-apple-tvos",
            "tvOS",
            "Release-appletvos",
        ),
    ];
    let selection = env::args().nth(1);
    if env::args().len() > 2
        || selection
            .as_ref()
            .is_some_and(|s| !targets.iter().any(|t| t.0 == s))
    {
        return Err("Supply ios-sim, ios, tvos-sim, tvos, or no argument for all".into());
    }
    let legal_tool = legal_resources::prepare(repo, &derived)?;
    for (name, scheme, triple, platform, products) in targets {
        if selection.as_ref().is_some_and(|s| s != name) {
            continue;
        }
        let library = core.join(triple);
        if !library.join("libbluefin_apple_core.a").is_file() {
            return Err(format!("Missing {triple} core").into());
        }
        let data = derived.join(name);
        let status = Command::new("xcodebuild")
            .args([
                "-project",
                project,
                "-scheme",
                scheme,
                "-configuration",
                "Release",
                "-jobs",
                "1",
                "-destination",
            ])
            .arg(format!("generic/platform={platform}"))
            .arg("-derivedDataPath")
            .arg(&data)
            .arg("CODE_SIGNING_ALLOWED=NO")
            .arg(format!("BLUEFIN_RUST_LIB_DIR={}", library.display()))
            .arg(format!(
                "BLUEFIN_LEGAL_RESOURCES_TOOL={}",
                legal_tool.tool.display()
            ))
            .arg(format!(
                "BLUEFIN_LEGAL_FILELIST_DIR={}",
                legal_tool.lists.display()
            ))
            .arg("build")
            .status()?;
        if !status.success() {
            return Err(format!("{name} build failed: {status}").into());
        }
        let destination = output.join(name);
        fs::create_dir_all(&destination)?;
        let bundle = format!("{scheme}.app");
        copy_tree(
            &data.join("Build/Products").join(products).join(&bundle),
            &destination.join(&bundle),
        )?;
        println!("Retained unsigned {name} application");
    }
    Ok(())
}
