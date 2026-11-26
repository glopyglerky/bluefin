use crate::{error, Result};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Selection {
    Watch,
    Rust { index: String, target: &'static str },
}

impl Selection {
    pub(crate) fn scope(&self) -> Option<(&str, &str)> {
        match self {
            Self::Watch => None,
            Self::Rust { index, target } => Some((index, target)),
        }
    }
}

// Read per-target Xcode settings. A command-line dependency selection can leak
// from an iPhone build into its embedded Watch target, so it is never consulted.
pub(crate) fn select(platform: &str, effective: &str, archs: &str) -> Result<Selection> {
    let architectures: Vec<_> = archs.split_whitespace().collect();
    if architectures.is_empty() {
        return Err(error("Xcode must supply the target ARCHS"));
    }
    if platform == "watchos" || platform == "watchsimulator" {
        let expected_effective = format!("-{platform}");
        if !effective.is_empty() && effective != expected_effective {
            return Err(error("Conflicting Watch platform settings"));
        }
        for (i, architecture) in architectures.iter().enumerate() {
            let supported =
                *architecture == "arm64" || (platform == "watchos" && *architecture == "arm64_32");
            if !supported || architectures[..i].contains(architecture) {
                return Err(error("Unqualified Watch architecture selection"));
            }
        }
        return Ok(Selection::Watch);
    }
    if architectures.len() != 1 {
        return Err(error("Build each qualified Rust architecture separately; a multi-architecture notice scope is not qualified"));
    }
    let architecture = architectures[0];
    let target = match (platform, effective, architecture) {
        ("iphoneos", "" | "-iphoneos", "arm64") => "aarch64-apple-ios",
        ("iphonesimulator", "" | "-iphonesimulator", "arm64") => "aarch64-apple-ios-sim",
        ("appletvos", "" | "-appletvos", "arm64") => "aarch64-apple-tvos",
        ("appletvsimulator", "" | "-appletvsimulator", "arm64") => "aarch64-apple-tvos-sim",
        ("xros", "" | "-xros", "arm64") => "aarch64-apple-visionos",
        ("xrsimulator", "" | "-xrsimulator", "arm64") => "aarch64-apple-visionos-sim",
        ("macosx", "-maccatalyst", "arm64") => "aarch64-apple-ios-macabi",
        ("macosx", "-maccatalyst", "x86_64") => "x86_64-apple-ios-macabi",
        _ => return Err(error("Unqualified Xcode platform or architecture")),
    };
    Ok(Selection::Rust {
        index: format!("apple-{target}"),
        target,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_eight_archive_scopes_match_the_exact_platform() {
        for (platform, effective, arch, target) in [
            ("iphoneos", "-iphoneos", "arm64", "aarch64-apple-ios"),
            (
                "iphonesimulator",
                "-iphonesimulator",
                "arm64",
                "aarch64-apple-ios-sim",
            ),
            ("appletvos", "-appletvos", "arm64", "aarch64-apple-tvos"),
            (
                "appletvsimulator",
                "-appletvsimulator",
                "arm64",
                "aarch64-apple-tvos-sim",
            ),
            ("xros", "-xros", "arm64", "aarch64-apple-visionos"),
            (
                "xrsimulator",
                "-xrsimulator",
                "arm64",
                "aarch64-apple-visionos-sim",
            ),
            (
                "macosx",
                "-maccatalyst",
                "arm64",
                "aarch64-apple-ios-macabi",
            ),
            (
                "macosx",
                "-maccatalyst",
                "x86_64",
                "x86_64-apple-ios-macabi",
            ),
        ] {
            let actual = select(platform, effective, arch).unwrap();
            assert_eq!(
                actual.scope(),
                Some((format!("apple-{target}").as_str(), target))
            );
            if platform != "macosx" {
                assert_eq!(select(platform, "", arch).unwrap(), actual);
            }
        }
    }

    #[test]
    fn watch_has_no_rust_scope_even_in_a_fat_device_build() {
        for (platform, archs) in [
            ("watchos", "arm64 arm64_32"),
            ("watchos", "arm64_32 arm64"),
            ("watchos", "arm64_32"),
            ("watchsimulator", "arm64"),
        ] {
            assert_eq!(
                select(platform, &format!("-{platform}"), archs).unwrap(),
                Selection::Watch
            );
        }
    }

    #[test]
    fn ambiguous_or_unqualified_selections_are_refused() {
        for (platform, effective, archs) in [
            ("", "", "arm64"),
            ("iphoneos", "-iphonesimulator", "arm64"),
            ("iphonesimulator", "-iphoneos", "arm64"),
            ("xros", "-xrsimulator", "arm64"),
            ("macosx", "", "arm64"),
            ("macosx", "-maccatalyst", "arm64 x86_64"),
            ("iphoneos", "-iphoneos", "arm64 arm64"),
            ("iphoneos", "-iphoneos", "arm64e"),
            ("iphonesimulator", "-iphonesimulator", "x86_64"),
            ("xrsimulator", "-xrsimulator", ""),
            ("watchsimulator", "-watchsimulator", "arm64_32"),
            ("watchos", "-iphoneos", "arm64"),
            ("watchos", "-watchos", "arm64 arm64"),
            ("watchos", "-watchos", "undefined_arch"),
        ] {
            assert!(
                select(platform, effective, archs).is_err(),
                "{platform}/{effective}/{archs}"
            );
        }
    }
}
