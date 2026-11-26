//! Checks Git-eligible source and commit metadata without printing matched data.
use std::{collections::BTreeSet, env, error::Error, fs, path::Path, process::Command};

fn portable_home(line: &str) -> bool {
    let text = line.replace('\\', "/").to_ascii_lowercase();
    for prefix in ["/users/", "/home/"] {
        for (at, rest) in text
            .match_indices(prefix)
            .map(|(at, _)| (at, &text[at + prefix.len()..]))
        {
            if at > 0 && !text[..at].ends_with("file://") {
                let previous = text.as_bytes()[at - 1];
                if !b" \t\r\n\"'([=:`".contains(&previous) {
                    continue;
                }
            }
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || "._-$".contains(*c))
                .collect();
            let tail = &rest[name.len()..];
            // MediaBrowser has a /Users/{id}/... API. A URL or API route is
            // not a filesystem home; private literal names remain covered
            // by the external deployment marker list as well.
            let child = tail
                .strip_prefix('/')
                .unwrap_or("")
                .split(['/', '"', '\'', '?', ' ', '}'])
                .next()
                .unwrap_or("");
            let user_api = prefix == "/users/"
                && (["forgotpassword", "authenticatebyname"].contains(&name.as_str())
                    || [
                        "items",
                        "views",
                        "playeditems",
                        "playingitems",
                        "favoriteitems",
                        "configuration",
                        "policy",
                        "password",
                        "images",
                        "groupingoptions",
                        "intros",
                        "suggestions",
                        "authenticate",
                    ]
                    .contains(&child));
            if !name.is_empty()
                && !child.is_empty()
                && !user_api
                && ![
                    "user",
                    "u",
                    "test",
                    "foo",
                    "bar",
                    "admin",
                    "media",
                    "jeff", // Synthetic path-replacement fixtures inherited upstream.
                    "username",
                    "example",
                    "runner",
                    "runneradmin",
                    "builder",
                    "developer",
                    "your-user",
                    "alice",
                    "bob",
                    "jellyfin",
                    "ferrofin",
                    "bluefin",
                    "$user",
                ]
                .contains(&name.as_str())
            {
                return true;
            }
        }
    }
    false
}

fn findings(bytes: &[u8], markers: &[String]) -> Vec<(usize, &'static str)> {
    // NUL stripping also recognizes UTF-16 ASCII paths in binary resources.
    let compact: Vec<_> = bytes.iter().copied().filter(|b| *b != 0).collect();
    let text = String::from_utf8_lossy(&compact);
    text.lines()
        .enumerate()
        .filter_map(|(line, value)| {
            let value = value.to_ascii_lowercase();
            // Historical generic home examples may be allowed, but must never
            // mask a stricter private identifier or coordination-link match.
            let reason = if markers.iter().any(|marker| value.contains(marker)) {
                Some("private-marker")
            } else if value.contains(concat!("t3", "-thread://")) {
                Some("private-coordination-link")
            } else if portable_home(&value) {
                Some("personal-home-path")
            } else {
                None
            };
            reason.map(|reason| (line + 1, reason))
        })
        .collect()
}

fn git(root: &Path, arguments: &[&str]) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut command = Command::new("git");
    // The supplied checkout is authoritative, not ambient Git routing/config.
    for (name, _) in env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            command.env_remove(name);
        }
    }
    let result = command.arg("-C").arg(root).args(arguments).output()?;
    if !result.status.success() {
        return Err("Git inventory or metadata inspection failed".into());
    }
    Ok(result.stdout)
}

fn workspace_artifact(path: &str) -> bool {
    path.split('/')
        .any(|part| part == ".DS_Store" || part == "xcuserdata" || part.ends_with(".xcuserstate"))
}

fn main() -> Result<(), Box<dyn Error>> {
    let root = env::args_os()
        .nth(1)
        .map_or_else(env::current_dir, |v| Ok(v.into()))?
        .canonicalize()?;
    let top = git(&root, &["rev-parse", "--show-toplevel"])?;
    let top = std::str::from_utf8(&top)?.trim_end_matches(['\r', '\n']);
    if Path::new(top).canonicalize()? != root {
        return Err("Run the privacy check at the repository root".into());
    }
    let mut markers = Vec::new();
    if let Some(path) = env::var_os("BLUEFIN_PRIVATE_MARKERS_FILE") {
        let path = Path::new(&path).canonicalize()?;
        if path.starts_with(&root) {
            return Err("Private marker configuration must stay outside the checkout".into());
        }
        for value in fs::read_to_string(path)?
            .lines()
            .map(str::trim)
            .filter(|v| !v.is_empty() && !v.starts_with('#'))
        {
            if value.len() < 4 {
                return Err("Private markers must contain at least four bytes".into());
            }
            markers.push(value.to_ascii_lowercase());
        }
    }
    let inventory = git(
        &root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    )?;
    let files: BTreeSet<_> = inventory
        .split(|b| *b == 0)
        .filter(|v| !v.is_empty())
        .map(std::str::from_utf8)
        .collect::<Result<_, _>>()?;
    let mut failed = false;
    let mut checked = 0;
    for relative in files {
        if workspace_artifact(relative) {
            eprintln!("<local-workspace-artifact>: private-workspace-data");
            failed = true;
            continue;
        }
        let private_path = !findings(relative.as_bytes(), &markers).is_empty();
        let display = if private_path {
            "<private-source-path>"
        } else {
            relative
        };
        if private_path {
            eprintln!("{display}: private-file-name");
            failed = true;
        }
        let path = Path::new(relative);
        if path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err("Git returned an unsafe source path".into());
        }
        let full = root.join(path);
        let metadata = match fs::symlink_metadata(&full) {
            Ok(v) => v,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue, // Working-tree deletion.
            Err(error) => return Err(error.into()),
        };
        if metadata.is_dir() {
            return Err("Submodule source needs a separate privacy check".into());
        }
        if metadata.len() > 32 * 1024 * 1024 {
            eprintln!("{display}: large-file-needs-review");
            failed = true;
            continue;
        }
        let data = if metadata.file_type().is_symlink() {
            match full.canonicalize() {
                Ok(target) if target.starts_with(&root) => {}
                _ => {
                    eprintln!("{display}: unresolved-or-outside-source-symlink");
                    failed = true;
                    continue;
                }
            }
            fs::read_link(&full)?
                .to_str()
                .ok_or("Non-UTF8 symlink")?
                .as_bytes()
                .to_vec()
        } else {
            fs::read(&full)?
        };
        for (line, reason) in findings(&data, &markers) {
            eprintln!("{display}:{line}: {reason}");
            failed = true;
        }
        checked += 1;
    }
    for (line, reason) in findings(
        &git(&root, &["log", "--all", "--format=%an%n%ae%n%B"])?,
        &markers,
    ) {
        if reason == "personal-home-path" {
            continue;
        } // Existing upstream commit examples.
        eprintln!("git-metadata:{line}: {reason}");
        failed = true;
    }
    for (line, reason) in findings(
        &git(&root, &["log", "-1", "--format=%an%n%ae%n%B"])?,
        &markers,
    ) {
        if reason == "personal-home-path" {
            eprintln!("current-commit:{line}: {reason}");
            failed = true;
        }
    }
    if failed {
        return Err("Public-source privacy check failed; matched values were withheld".into());
    }
    println!(
        "Public-source privacy check passed: {checked} Git-eligible files and local-ref commit metadata"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catches_personal_paths_across_platforms() {
        for prefix in ["/Users/", "/home/", "C:\\Users\\"] {
            let value = format!("{prefix}private-test-account/projects/file");
            assert_eq!(findings(value.as_bytes(), &[])[0].1, "personal-home-path");
        }
    }
    #[test]
    fn accepts_portable_examples_and_runtime_paths() {
        for value in [
            "/home/user/project",
            "/Users/runner/project",
            "/proc/self/fd/0",
            "https://example.invalid/library",
        ] {
            assert!(findings(value.as_bytes(), &[]).is_empty());
        }
    }
    #[test]
    fn media_api_routes_are_not_home_directories() {
        for route in [
            "/Users/AuthenticateByName",
            "/Users/fixture-viewer/Items",
            "/Users/fixture-viewer/PlayedItems/123",
            "/Users/Public",
        ] {
            assert!(findings(route.as_bytes(), &[]).is_empty(), "{route}");
        }
        for route in [
            "config/users/named/encoding.json",
            "{config}/users/named/profile",
            "/Users/ForgotPassword/Pin",
            "/Users/fixture-viewer/Authenticate?pw=example",
        ] {
            assert!(findings(route.as_bytes(), &[]).is_empty(), "{route}");
        }
    }
    #[test]
    fn checks_binary_resources_case_insensitively() {
        let value = b"\0I\0N\0T\0E\0R\0N\0A\0L\0";
        assert_eq!(
            findings(value, &["internal".into()]),
            [(1, "private-marker")]
        );
    }
    #[test]
    fn catches_private_links_without_echoing_values() {
        let value = format!("{}-thread://private-example", "t3");
        assert_eq!(
            findings(value.as_bytes(), &[]),
            [(1, "private-coordination-link")]
        );
    }
    #[test]
    fn reports_locations_only() {
        assert_eq!(
            findings(b"generic\nexample-secret\n", &["example-secret".into()]),
            [(2, "private-marker")]
        );
    }
    #[test]
    fn excludes_private_workspace_files() {
        assert!(workspace_artifact(
            "project/xcuserdata/example.xcuserdatad/state"
        ));
        assert!(workspace_artifact("assets/.DS_Store"));
        assert!(workspace_artifact("project/window.xcuserstate"));
        assert!(!workspace_artifact(
            "project/xcshareddata/schemes/App.xcscheme"
        ));
    }
}
