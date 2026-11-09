//! Packaging boundary check. This tool is never linked into a client.
use std::{env, error::Error, fs, path::Path, process::Command};

const LOCAL_MANIFESTS: &[&str] = &[
    "apps/bluefin-apple/rust/Cargo.toml",
    "apps/bluefin-apple/iroh-modern/Cargo.toml",
    "apps/bluefin-apple/iroh-modern/vendor/netdev-0.46.3/Cargo.toml",
    "apps/bluefin-apple/iroh-modern/vendor/netwatch-0.19.3/Cargo.toml",
    "crates/bluefin-contract/Cargo.toml",
];
const SOURCE_ROOTS: &[&str] = &[
    "apps/bluefin-apple/Sources",
    "apps/bluefin-apple/include",
    "apps/bluefin-apple/rust/src",
    "apps/bluefin-apple/iroh-modern/src",
    "crates/bluefin-contract/src",
];
const FORBIDDEN: &[&str] = &[
    "x-api-key",
    "/api/v1/search",
    "/api/v1/request",
    "/api/v1/user",
];

fn has_service_name(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    ["seerr", "jellyseerr", "overseerr"].iter().any(|name| {
        lower.match_indices(name).any(|(index, _)| {
            index == 0
                || !text.as_bytes()[index - 1].is_ascii_alphanumeric()
                || (text.as_bytes()[index].is_ascii_uppercase()
                    && text.as_bytes()[index - 1].is_ascii_lowercase())
        })
    })
}

fn check_text(text: &str) -> Result<(), &'static str> {
    if has_service_name(text) {
        return Err("server discovery service");
    }
    let lower = text.to_ascii_lowercase();
    FORBIDDEN
        .iter()
        .copied()
        .find(|pattern| lower.contains(pattern))
        .map_or(Ok(()), Err)
}

fn check_sources(path: &Path) -> Result<usize, Box<dyn Error>> {
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(format!("Native source must not be a symlink: {}", path.display()).into());
    }
    if path.is_dir() {
        return fs::read_dir(path)?
            .try_fold(0, |count, entry| Ok(count + check_sources(&entry?.path())?));
    }
    let text = fs::read_to_string(path)?;
    check_text(&text).map_err(|pattern| {
        format!(
            "Server integration marker {pattern:?} in native source {}",
            path.display()
        )
    })?;
    Ok(1)
}

fn check_packages(repo: &Path, metadata: &serde_json::Value) -> Result<usize, Box<dyn Error>> {
    if !metadata["resolve"]["nodes"].is_array() {
        return Err("Cargo must return a resolved dependency graph".into());
    }
    let packages = metadata["packages"]
        .as_array()
        .ok_or("Cargo packages missing")?;
    if packages.is_empty() {
        return Err("Cargo returned no packages".into());
    }
    for package in packages {
        let name = package["name"].as_str().ok_or("Package name missing")?;
        if has_service_name(name) {
            return Err(format!("Server integration package in native graph: {name}").into());
        }
        if package["source"].is_null() {
            let manifest = Path::new(
                package["manifest_path"]
                    .as_str()
                    .ok_or("Manifest missing")?,
            )
            .canonicalize()?;
            let allowed = LOCAL_MANIFESTS.iter().any(|relative| {
                repo.join(relative)
                    .canonicalize()
                    .is_ok_and(|path| path == manifest)
            });
            if !allowed {
                return Err(format!("Unapproved local dependency in native graph: {name}").into());
            }
        }
    }
    Ok(packages.len())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let repo = args.next().ok_or("Supply the repository root")?;
    if args.next().is_some() {
        return Err("Supply only the repository root".into());
    }
    let repo = Path::new(&repo).canonicalize()?;
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--locked",
            "--all-features",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(repo.join(LOCAL_MANIFESTS[0]))
        .output()?;
    if !output.status.success() {
        return Err("Native Cargo graph resolution failed; inspect cargo metadata locally".into());
    }
    let packages = check_packages(&repo, &serde_json::from_slice(&output.stdout)?)?;
    let mut sources = 0;
    for root in SOURCE_ROOTS {
        sources += check_sources(&repo.join(root))?;
    }
    println!(
        "Native boundary passed: {packages} resolved packages, {sources} source files; no server connector dependency or direct service markers."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn direct_service_calls_and_renamed_local_dependencies_are_rejected() {
        for text in ["JellyseerrClient", "Seerr", "X-Api-Key", "/api/v1/request"] {
            assert!(check_text(text).is_err(), "{text}");
        }
        assert!(check_text("/Bluefin/Discovery/Search").is_ok());
        assert!(check_text("CrlParseError").is_ok());
        assert!(check_text("createSeerrClient").is_err());
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .unwrap();
        let graph = json!({"resolve":{"nodes":[]}, "packages":[{
            "name":"innocent-name", "source":null,
            "manifest_path":repo.join("crates/ferrofin-api/Cargo.toml")
        }]});
        assert!(check_packages(repo, &graph).is_err());
    }

    #[test]
    fn registry_connector_and_unresolved_graph_are_rejected() {
        let graph = json!({"resolve":{"nodes":[]}, "packages":[{
            "name":"seerr-sdk", "source":"registry+https://example.com"
        }]});
        assert!(check_packages(Path::new("."), &graph).is_err());
        assert!(check_packages(Path::new("."), &json!({"packages":[]})).is_err());
    }
}
