//! Real Git/CLI regressions. All identities and history are disposable fixtures.
use std::{env, error::Error, fs, path::Path, process::Command};

fn git(root: &Path, hooks: &Path, args: &[&str]) -> Result<(), Box<dyn Error>> {
    let mut command = Command::new("git");
    // Fixtures must never inherit a caller's repository, index or Git config.
    for (name, _) in env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            command.env_remove(name);
        }
    }
    let result = command
        .args([
            "-c",
            "commit.gpgSign=false",
            "-c",
            "user.name=Fixture Author",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
        ])
        .arg(format!("core.hooksPath={}", hooks.display()))
        .arg("-C")
        .arg(root)
        .args(args)
        .output()?;
    if !result.status.success() {
        return Err("Synthetic Git fixture command failed".into());
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    if arguments.len() != 2 {
        return Err("Expected CHECK_EXECUTABLE FRESH_OUTPUT_DIRECTORY".into());
    }
    let executable = Path::new(&arguments[0]).canonicalize()?;
    let output = Path::new(&arguments[1]);
    fs::create_dir(output)?; // Refuse any pre-existing directory, including symlinks.
    let output = output.canonicalize()?;
    let hooks = output.join("empty-hooks");
    fs::create_dir(&hooks)?;
    let marker = "synthetic-owner-private";
    let markers = output.join("external-markers.txt");
    fs::write(&markers, format!("{marker}\n"))?;
    let home = format!("/{}/{}/projects/file", "home", "synthetic-viewer");
    let link = format!("{}-thread://synthetic-example", "t3");
    let cases = [
        ("old-home-control", home.clone(), true, false),
        ("current-home-control", home.clone(), false, true),
        (
            "old-private-and-home",
            format!("{marker} {home}"),
            true,
            true,
        ),
        ("old-link-and-home", format!("{link} {home}"), true, true),
    ];
    let mut failures = 0;
    for (name, message, clean_tip, must_reject) in cases {
        let root = output.join(name);
        fs::create_dir(&root)?;
        git(&root, &hooks, &["init", "-q"])?;
        fs::write(root.join("clean.txt"), "Portable synthetic source\n")?;
        git(&root, &hooks, &["add", "clean.txt"])?;
        git(&root, &hooks, &["commit", "-qm", "Initial fixture"])?;
        git(&root, &hooks, &["commit", "--allow-empty", "-qm", &message])?;
        if clean_tip {
            git(
                &root,
                &hooks,
                &["commit", "--allow-empty", "-qm", "Clean current commit"],
            )?;
        }
        let result = Command::new(&executable)
            .arg(&root)
            .env("BLUEFIN_PRIVATE_MARKERS_FILE", &markers)
            .output()?;
        let diagnostics = [result.stdout, result.stderr].concat();
        let text = String::from_utf8_lossy(&diagnostics);
        let passed = !result.status.success() == must_reject
            && !text.contains(marker)
            && !text.contains(&link);
        println!("{} {name}", if passed { "PASS" } else { "FAIL" });
        if !passed {
            failures += 1;
        }
    }
    let clean = output.join("old-home-control");
    let private = output.join("private-working-tree");
    fs::create_dir(&private)?;
    git(&private, &hooks, &["init", "-q"])?;
    git(
        &private,
        &hooks,
        &["commit", "--allow-empty", "-qm", "Clean fixture"],
    )?;
    fs::write(private.join("private.txt"), marker)?;
    let result = Command::new(&executable)
        .arg(&private)
        .env("BLUEFIN_PRIVATE_MARKERS_FILE", &markers)
        .env("GIT_DIR", clean.join(".git"))
        .env("GIT_WORK_TREE", &clean)
        .env("GIT_INDEX_FILE", clean.join(".git/index"))
        .output()?;
    let text = String::from_utf8_lossy(&result.stderr);
    let passed =
        !result.status.success() && text.contains("private-marker") && !text.contains(marker);
    println!(
        "{} ambient-git-context",
        if passed { "PASS" } else { "FAIL" }
    );
    failures += usize::from(!passed);

    let nested = private.join("nested");
    fs::create_dir(&nested)?;
    let result = Command::new(&executable).arg(&nested).output()?;
    let passed = !result.status.success();
    println!(
        "{} repository-root-required",
        if passed { "PASS" } else { "FAIL" }
    );
    failures += usize::from(!passed);
    // Only the fresh directory this process created is removed, after Git/CLI exit.
    fs::remove_dir_all(&output)?;
    if failures != 0 {
        return Err("Privacy CLI history regression failed".into());
    }
    Ok(())
}
