use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs, io,
    path::{Component, Path, PathBuf},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
mod filelists;
mod xcode;
const MANIFEST: &str = "bluefin-legal-manifest.json";
const HTML: [&str; 2] = [
    "90567e2718bf7fd65a71a3a43c5596488e80e5f51ed02bfea6fec54458b5f3d1",
    "5647be074c8edf7339fd863055923a8fc80bc5610a8d4661ec3b767b9d392c27",
];

fn error(message: &str) -> Box<dyn std::error::Error> {
    io::Error::new(io::ErrorKind::InvalidData, message).into()
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| error("Missing notice string"))
}
fn array<'a>(value: &'a Value, key: &str) -> Result<&'a [Value]> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| error("Missing notice array"))
}
fn resource(root: &Path, base: &Path, reference: &str) -> Result<PathBuf> {
    if reference.is_empty()
        || reference.len() > 4096
        || reference.starts_with('/')
        || reference
            .bytes()
            .any(|b| b < 32 || b == 127 || b":\\%?#".contains(&b))
    {
        return Err(error("Invalid local notice reference"));
    }
    if !base.starts_with(root) {
        return Err(error("Notice base is outside its source root"));
    }
    let mut requested = base.to_owned();
    for component in Path::new(reference).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if requested == root {
                    return Err(error("Notice reference leaves its source root"));
                }
                requested.pop();
            }
            Component::Normal(name) => {
                let mut exact_name = false;
                for entry in fs::read_dir(&requested)? {
                    if entry?.file_name() == name {
                        exact_name = true;
                        break;
                    }
                }
                if !exact_name {
                    return Err(error("Notice reference does not match a source filename"));
                }
                requested.push(name);
                if fs::symlink_metadata(&requested)?.file_type().is_symlink() {
                    return Err(error("Referenced notices cannot contain symlinks"));
                }
            }
            _ => return Err(error("Invalid local notice component")),
        }
    }
    let file = requested.canonicalize()?;
    let metadata = file.metadata()?;
    if !file.starts_with(root)
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > 4 * 1024 * 1024
    {
        return Err(error(
            "Notice must be a bounded regular file inside its resource root",
        ));
    }
    Ok(file)
}
fn admit(
    root: &Path,
    base: &Path,
    reference: &str,
    files: &mut BTreeMap<PathBuf, Vec<u8>>,
) -> Result<PathBuf> {
    let file = resource(root, base, reference)?;
    files.insert(file.strip_prefix(root)?.to_owned(), fs::read(&file)?);
    if files.values().map(Vec::len).sum::<usize>() > 32 * 1024 * 1024 {
        return Err(error("Legal resource scope exceeds its byte budget"));
    }
    Ok(file)
}
fn plan(root: &Path, index: Option<(&str, &str)>) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    let mut files = BTreeMap::new();
    admit(root, root, "first-party/LICENSE", &mut files)?;
    admit(root, root, "first-party/README.md", &mut files)?;
    if let Some((name, target)) = index {
        let index_file = admit(root, root, &format!("indexes/{name}.json"), &mut files)?;
        let value: Value = serde_json::from_slice(&fs::read(&index_file)?)?;
        if text(&value, "platform")? != "apple" || text(&value, "target")? != target {
            return Err(error(
                "Legal index does not match the selected Apple target",
            ));
        }
        let base = index_file
            .parent()
            .ok_or_else(|| error("Missing index parent"))?;
        let first = array(&value, "first_party")?;
        if first.is_empty() || first.len() > 64 {
            return Err(error("Invalid first-party scope"));
        }
        for reference in first {
            admit(
                root,
                base,
                reference
                    .as_str()
                    .ok_or_else(|| error("Invalid notice path"))?,
                &mut files,
            )?;
        }
        let standard = admit(
            root,
            base,
            text(&value, "standard_library_html")?,
            &mut files,
        )?;
        if !HTML.contains(&digest(&fs::read(standard)?).as_str()) {
            return Err(error(
                "Standard-library HTML has not been qualified for native reading",
            ));
        }
        let packages = array(&value, "packages")?;
        if packages.is_empty() || packages.len() > 4096 {
            return Err(error("Invalid runtime package scope"));
        }
        for package in packages {
            let references = array(package, "texts")?;
            if references.len() > 64 {
                return Err(error("Invalid package document count"));
            }
            for reference in references {
                admit(
                    root,
                    base,
                    reference
                        .as_str()
                        .ok_or_else(|| error("Invalid package notice path"))?,
                    &mut files,
                )?;
            }
        }
        admit(root, root, "third-party/text-provenance.json", &mut files)?;
    }
    Ok(files)
}
fn regular_files(
    root: &Path,
    directory: &Path,
    files: &mut BTreeMap<PathBuf, Vec<u8>>,
) -> Result<()> {
    check_mode(directory, 0o755)?;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(error("Staged notices cannot contain symlinks"));
        }
        if kind.is_dir() {
            regular_files(root, &entry.path(), files)?;
        } else if kind.is_file() {
            check_mode(&entry.path(), 0o644)?;
            files.insert(
                entry.path().strip_prefix(root)?.to_owned(),
                fs::read(entry.path())?,
            );
        } else {
            return Err(error("Unexpected staged resource type"));
        }
    }
    Ok(())
}
#[cfg(unix)]
fn check_mode(path: &Path, expected: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if path.metadata()?.permissions().mode() & 0o7777 != expected {
        return Err(error(
            "Staged notice modes differ; use a fresh build destination",
        ));
    }
    Ok(())
}
#[cfg(not(unix))]
fn check_mode(_: &Path, _: u32) -> Result<()> {
    Err(error(
        "Apple legal staging requires a qualified Unix build host",
    ))
}
#[cfg(unix)]
fn overlaps_by_identity(source: &Path, output: &Path) -> Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let source_id = source.metadata()?;
    for ancestor in output.ancestors() {
        match ancestor.metadata() {
            Ok(value) if value.dev() == source_id.dev() && value.ino() == source_id.ino() => {
                return Ok(true)
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    if let Ok(output_id) = output.metadata() {
        for ancestor in source.ancestors() {
            let value = ancestor.metadata()?;
            if value.dev() == output_id.dev() && value.ino() == output_id.ino() {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
#[cfg(not(unix))]
fn overlaps_by_identity(_: &Path, _: &Path) -> Result<bool> {
    Err(error(
        "Apple legal staging requires a qualified Unix build host",
    ))
}
// Xcode creates the parent directories of declared output files before running
// the phase. Only an empty, expected directory skeleton may be reclaimed.
// rmdir never removes a file introduced during a concurrent change.
fn empty_layout(root: &Path, files: &BTreeMap<PathBuf, Vec<u8>>) -> Result<Vec<PathBuf>> {
    let allowed: BTreeSet<PathBuf> = files
        .keys()
        .flat_map(|p| {
            p.parent()
                .into_iter()
                .flat_map(Path::ancestors)
                .map(Path::to_owned)
        })
        .collect();
    fn inspect(
        root: &Path,
        directory: &Path,
        allowed: &BTreeSet<PathBuf>,
        found: &mut Vec<PathBuf>,
    ) -> Result<()> {
        let kind = fs::symlink_metadata(directory)?.file_type();
        if !kind.is_dir() || kind.is_symlink() || !allowed.contains(directory.strip_prefix(root)?) {
            return Err(error(
                "Existing build directory is not an expected empty layout",
            ));
        }
        check_mode(directory, 0o755)?;
        for entry in fs::read_dir(directory)? {
            inspect(root, &entry?.path(), allowed, found)?;
        }
        found.push(directory.to_owned());
        Ok(())
    }
    let mut directories = Vec::new();
    inspect(root, root, &allowed, &mut directories)?;
    Ok(directories)
}
fn stage(root: &Path, output: &Path, selection: Option<(&str, &str)>) -> Result<usize> {
    if !output.is_absolute()
        || output
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(error(
            "A fresh absolute build-resource directory is required",
        ));
    }
    let root = root.canonicalize()?;
    let mut ancestor = PathBuf::new();
    for part in output.components() {
        ancestor.push(part);
        if let Ok(metadata) = fs::symlink_metadata(&ancestor) {
            if metadata.file_type().is_symlink() {
                return Err(error("Build-resource destination has a symlink ancestor"));
            }
        }
    }
    if output.starts_with(&root) || root.starts_with(output) || overlaps_by_identity(&root, output)?
    {
        return Err(error("Resource output must be separate from source"));
    }
    let mut files = plan(&root, selection)?;
    let receipts: BTreeMap<_, _> = files
        .iter()
        .map(|(path, bytes)| (path.to_string_lossy().into_owned(), digest(bytes)))
        .collect();
    let manifest = json!({"target": selection.map(|(_, target)| target), "index": selection.map(|(name, _)| name), "files": receipts});
    files.insert(MANIFEST.into(), serde_json::to_vec_pretty(&manifest)?);
    let mut already_staged = false;
    let mut empty_output = Vec::new();
    if output.exists() {
        let mut present = BTreeMap::new();
        regular_files(output, output, &mut present)?;
        if present == files {
            already_staged = true;
        } else if present.is_empty() {
            empty_output = empty_layout(output, &files)?;
        } else {
            return Err(error(
                "Existing legal output differs; use a fresh build destination",
            ));
        }
    }
    let parent = output
        .parent()
        .ok_or_else(|| error("Missing build output parent"))?;
    fs::create_dir_all(parent)?;
    // A declared sibling output lets Xcode permit this exact temporary tree
    // without granting the script write access to the rest of the app bundle.
    // create_dir refuses leftover or concurrent staging ownership.
    let temporary = parent.join(".bluefin-legal-stage");
    if temporary.exists() {
        for directory in empty_layout(&temporary, &files)? {
            fs::remove_dir(directory)?;
        }
    }
    if already_staged {
        return Ok(files.len());
    }
    fs::create_dir(&temporary)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o755))?;
    }
    let result = (|| -> Result<()> {
        for (relative, bytes) in &files {
            let destination = temporary.join(relative);
            fs::create_dir_all(
                destination
                    .parent()
                    .ok_or_else(|| error("Missing resource parent"))?,
            )?;
            fs::write(&destination, bytes)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&destination, fs::Permissions::from_mode(0o644))?;
                let mut directory = destination.parent();
                while let Some(path) = directory {
                    if path == temporary {
                        break;
                    }
                    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
                    directory = path.parent();
                }
            }
        }
        for directory in &empty_output {
            fs::remove_dir(directory)?;
        }
        fs::rename(&temporary, output)?;
        Ok(())
    })();
    if temporary.exists() {
        fs::remove_dir_all(&temporary)?;
    }
    result?;
    Ok(files.len())
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--xcode-filelists") {
        if args.len() != 3 {
            return Err(error(
                "Usage: legal-resources --xcode-filelists SOURCE OUTPUT",
            ));
        }
        let count = filelists::prepare(Path::new(&args[1]), Path::new(&args[2]))?;
        println!("Verified {count} target-specific Xcode file lists");
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("--xcode") {
        if args.len() != 3 {
            return Err(error("Usage: legal-resources --xcode SOURCE OUTPUT"));
        }
        let selection = xcode::select(
            &env::var("PLATFORM_NAME")?,
            &env::var("EFFECTIVE_PLATFORM_NAME")?,
            &env::var("ARCHS")?,
        )?;
        let count = stage(Path::new(&args[1]), Path::new(&args[2]), selection.scope())?;
        println!("Verified and staged {count} target-specific local legal resources");
        return Ok(());
    }
    if args.len() != 4 {
        return Err(error("Usage: legal-resources SOURCE INDEX TARGET OUTPUT; Watch uses '-' for INDEX and TARGET"));
    }
    let selection = match (args[1].as_str(), args[2].as_str()) {
        ("-", "-") => None,
        ("-", _) | (_, "-") => return Err(error("Incomplete legal target selection")),
        (name, target) => Some((name, target)),
    };
    let count = stage(Path::new(&args[0]), Path::new(&args[3]), selection)?;
    println!("Verified and staged {count} local legal resources");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    struct Case(PathBuf);
    impl Case {
        fn new() -> Self {
            let root = PathBuf::from(
                env::var_os("BLUEFIN_LEGAL_TEST_ROOT")
                    .expect("Set a bounded external BLUEFIN_LEGAL_TEST_ROOT"),
            );
            let case = root.join(format!(
                "case-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&case).unwrap();
            Self(case)
        }
        fn source(&self) -> PathBuf {
            let source = self.0.join("source");
            fs::create_dir_all(source.join("first-party")).unwrap();
            fs::write(
                source.join("first-party/LICENSE"),
                b"Original synthetic license text",
            )
            .unwrap();
            fs::write(
                source.join("first-party/README.md"),
                b"Original synthetic attribution",
            )
            .unwrap();
            source
        }
    }
    impl Drop for Case {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    #[test]
    fn watch_scope_and_idempotence_preserve_original_bytes() {
        let case = Case::new();
        let source = case.source();
        let output = case.0.join("Legal");
        assert_eq!(stage(&source, &output, None).unwrap(), 3);
        assert_eq!(stage(&source, &output, None).unwrap(), 3);
        assert_eq!(
            fs::read(source.join("first-party/LICENSE")).unwrap(),
            fs::read(output.join("first-party/LICENSE")).unwrap()
        );
        assert!(!output.join("third-party").exists());
    }
    #[test]
    fn unexpected_or_changed_output_is_not_overwritten() {
        let case = Case::new();
        let source = case.source();
        let output = case.0.join("Legal");
        stage(&source, &output, None).unwrap();
        fs::write(output.join("unmanaged"), b"Must remain").unwrap();
        assert!(stage(&source, &output, None).is_err());
        assert_eq!(fs::read(output.join("unmanaged")).unwrap(), b"Must remain");
    }
    #[test]
    fn existing_staging_ownership_is_not_adopted_or_deleted() {
        let case = Case::new();
        let source = case.source();
        let temporary = case.0.join(".bluefin-legal-stage");
        fs::create_dir(&temporary).unwrap();
        fs::write(temporary.join("owner"), b"Must remain").unwrap();
        let output = case.0.join("Legal");
        assert!(stage(&source, &output, None).is_err());
        assert_eq!(fs::read(temporary.join("owner")).unwrap(), b"Must remain");
        assert!(!output.exists());
    }
    #[test]
    fn xcode_empty_output_parents_can_be_staged_but_unknown_layouts_are_retained() {
        let case = Case::new();
        let source = case.source();
        let output = case.0.join("Legal");
        let temporary = case.0.join(".bluefin-legal-stage");
        fs::create_dir_all(output.join("first-party")).unwrap();
        fs::create_dir_all(temporary.join("first-party")).unwrap();
        assert_eq!(stage(&source, &output, None).unwrap(), 3);
        assert!(!temporary.exists());
        fs::create_dir_all(temporary.join("first-party")).unwrap();
        assert_eq!(stage(&source, &output, None).unwrap(), 3);
        assert!(!temporary.exists());
        let unknown = case.0.join("Other");
        fs::create_dir_all(unknown.join("unmanaged")).unwrap();
        assert!(stage(&source, &unknown, None).is_err());
        assert!(unknown.join("unmanaged").is_dir());
    }
    #[test]
    fn source_and_output_must_not_overlap() {
        let case = Case::new();
        let source = case.source();
        assert!(stage(&source, &source.join("Legal"), None).is_err());
        assert!(stage(&source, &case.0, None).is_err());
        assert!(!source.join("Legal").exists());
    }
    #[cfg(unix)]
    #[test]
    fn case_alias_cannot_stage_into_the_source_tree() {
        let case = Case::new();
        let source = case.source();
        let alias = case.0.join("SOURCE");
        if alias.is_dir() {
            assert!(stage(&source, &alias.join("Legal"), None).is_err());
            assert!(!source.join("Legal").exists());
        }
    }
    #[test]
    fn invalid_reference_shapes_and_directories_are_refused() {
        let case = Case::new();
        let source = case.source().canonicalize().unwrap();
        for value in [
            "",
            "/etc/passwd",
            "../outside",
            "file:test",
            "a%2fb",
            "a?b",
            "a#b",
            "a\\b",
            "a\u{7f}b",
            "first-party",
        ] {
            assert!(resource(&source, &source, value).is_err());
        }
    }
    #[test]
    fn absent_empty_and_oversized_documents_are_refused() {
        let case = Case::new();
        let source = case.source();
        let license = source.join("first-party/LICENSE");
        fs::remove_file(&license).unwrap();
        assert!(plan(&source, None).is_err());
        fs::write(&license, []).unwrap();
        assert!(plan(&source, None).is_err());
        fs::write(&license, vec![65; 4 * 1024 * 1024 + 1]).unwrap();
        assert!(plan(&source, None).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn input_escape_and_output_symlinks_are_refused() {
        use std::os::unix::fs::symlink;
        let case = Case::new();
        let source = case.source();
        let license = source.join("first-party/LICENSE");
        fs::remove_file(&license).unwrap();
        fs::write(case.0.join("outside"), b"Outside resource root").unwrap();
        symlink(case.0.join("outside"), &license).unwrap();
        assert!(plan(&source, None).is_err());
        fs::remove_file(&license).unwrap();
        fs::write(license, b"Original text").unwrap();
        symlink(&source, case.0.join("alias")).unwrap();
        assert!(stage(&source, &case.0.join("alias/Legal"), None).is_err());
        assert!(!source.join("Legal").exists());
    }
    #[test]
    fn wrong_target_does_not_stage_a_different_platform() {
        let case = Case::new();
        let source = case.source();
        fs::create_dir(source.join("indexes")).unwrap();
        fs::write(
            source.join("indexes/selection.json"),
            br#"{"platform":"apple","target":"other-target"}"#,
        )
        .unwrap();
        let output = case.0.join("Legal");
        assert!(stage(&source, &output, Some(("selection", "expected-target"))).is_err());
        assert!(!output.exists());
    }
    #[cfg(unix)]
    #[test]
    fn in_root_symlink_must_not_omit_reader_required_names() {
        use std::os::unix::fs::symlink;
        let case = Case::new();
        let source = case.source();
        fs::rename(
            source.join("first-party/LICENSE"),
            source.join("first-party/actual.txt"),
        )
        .unwrap();
        symlink("actual.txt", source.join("first-party/LICENSE")).unwrap();
        let output = case.0.join("Legal");
        assert!(
            stage(&source, &output, None).is_err(),
            "symlinked source must be refused before successful incomplete publication"
        );
        assert!(!output.exists());
    }
    #[cfg(unix)]
    #[test]
    fn repeated_stage_refuses_executable_notice_modes() {
        use std::os::unix::fs::PermissionsExt;
        let case = Case::new();
        let source = case.source();
        let output = case.0.join("Legal");
        stage(&source, &output, None).unwrap();
        let document = output.join("first-party/LICENSE");
        fs::set_permissions(&document, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            stage(&source, &output, None).is_err(),
            "changed resource modes must not be accepted"
        );
        assert_eq!(
            document.metadata().unwrap().permissions().mode() & 0o7777,
            0o755
        );
    }
    #[cfg(unix)]
    #[test]
    fn in_root_directory_symlinks_are_also_refused() {
        use std::os::unix::fs::symlink;
        let case = Case::new();
        let source = case.source();
        fs::rename(source.join("first-party"), source.join("originals")).unwrap();
        symlink("originals", source.join("first-party")).unwrap();
        assert!(stage(&source, &case.0.join("Legal"), None).is_err());
        assert!(!case.0.join("Legal").exists());
    }
    #[cfg(unix)]
    #[test]
    fn repeated_stage_refuses_changed_directory_modes() {
        use std::os::unix::fs::PermissionsExt;
        let case = Case::new();
        let source = case.source();
        let output = case.0.join("Legal");
        stage(&source, &output, None).unwrap();
        let directory = output.join("first-party");
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(stage(&source, &output, None).is_err());
        assert_eq!(
            directory.metadata().unwrap().permissions().mode() & 0o7777,
            0o700
        );
    }
}
