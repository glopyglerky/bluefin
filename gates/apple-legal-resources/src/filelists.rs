use crate::{error, overlaps_by_identity, plan, regular_files, Result, MANIFEST};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};

fn paths(files: &BTreeMap<PathBuf, Vec<u8>>) -> BTreeSet<PathBuf> {
    let mut result = BTreeSet::from([PathBuf::new()]);
    for path in files.keys() {
        for parent in path.ancestors() {
            result.insert(parent.to_owned());
        }
    }
    result
}

fn lists(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    let scopes: [(&str, &[&str]); 9] = [
        ("iphoneos", &["aarch64-apple-ios"]),
        ("iphonesimulator", &["aarch64-apple-ios-sim"]),
        ("appletvos", &["aarch64-apple-tvos"]),
        ("appletvsimulator", &["aarch64-apple-tvos-sim"]),
        ("xros", &["aarch64-apple-visionos"]),
        ("xrsimulator", &["aarch64-apple-visionos-sim"]),
        (
            "macosx",
            &["aarch64-apple-ios-macabi", "x86_64-apple-ios-macabi"],
        ),
        ("watchos", &[]),
        ("watchsimulator", &[]),
    ];
    let mut result = BTreeMap::new();
    for (platform, targets) in scopes {
        let mut files = BTreeMap::new();
        if targets.is_empty() {
            files = plan(root, None)?;
        }
        for target in targets {
            files.extend(plan(root, Some((&format!("apple-{target}"), target)))?);
        }
        let inputs = paths(&files)
            .into_iter()
            .map(|relative| format!("{}\n", root.join(relative).display()))
            .collect::<String>();
        files.insert(MANIFEST.into(), Vec::new());
        let mut outputs = String::new();
        for directory in ["Legal", ".bluefin-legal-stage"] {
            for relative in paths(&files) {
                let suffix = if relative.as_os_str().is_empty() {
                    String::new()
                } else {
                    format!("/{}", relative.display())
                };
                outputs.push_str(&format!("$(TARGET_BUILD_DIR)/$(UNLOCALIZED_RESOURCES_FOLDER_PATH)/{directory}{suffix}\n"));
            }
        }
        result.insert(
            format!("{platform}.inputs.xcfilelist").into(),
            inputs.into_bytes(),
        );
        result.insert(
            format!("{platform}.outputs.xcfilelist").into(),
            outputs.into_bytes(),
        );
    }
    Ok(result)
}

pub(crate) fn prepare(root: &Path, output: &Path) -> Result<usize> {
    let root = root.canonicalize()?;
    if !output.is_absolute()
        || output
            .components()
            .any(|c| matches!(c, Component::ParentDir))
        || root.to_string_lossy().contains(['\n', '\r', '$'])
        || output.starts_with(&root)
        || root.starts_with(output)
        || overlaps_by_identity(&root, output)?
    {
        return Err(error(
            "File lists require separate, unambiguous build paths",
        ));
    }
    for ancestor in output.ancestors() {
        if fs::symlink_metadata(ancestor).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(error("File-list destination has a symlink ancestor"));
        }
    }
    let files = lists(&root)?;
    if output.exists() {
        let mut present = BTreeMap::new();
        regular_files(output, output, &mut present)?;
        if files != present {
            return Err(error(
                "File lists changed; use a fresh host build destination",
            ));
        }
        return Ok(files.len());
    }
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(output)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(output, fs::Permissions::from_mode(0o755))?;
    }
    for (relative, bytes) in &files {
        let destination = output.join(relative);
        fs::write(&destination, bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(destination, fs::Permissions::from_mode(0o644))?;
        }
    }
    Ok(files.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flattened_paths_include_files_and_each_required_directory() {
        let files = BTreeMap::from([(PathBuf::from("first-party/LICENSE"), Vec::new())]);
        assert_eq!(
            paths(&files),
            BTreeSet::from([
                PathBuf::new(),
                PathBuf::from("first-party"),
                PathBuf::from("first-party/LICENSE")
            ])
        );
    }
}
