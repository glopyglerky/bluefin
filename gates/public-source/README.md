# Public-source privacy check

Compile this dependency-free Rust check outside the checkout, then run it from
the repository root:

```sh
rustc --edition=2024 -D warnings gates/public-source/main.rs -o "$BLUEFIN_OUTPUT/public-source"
"$BLUEFIN_OUTPUT/public-source" .
```

It inspects current Git-eligible tracked and untracked files, symlink values,
binary resources and author/email/message metadata reachable through local Git
refs. It rejects personal home-directory file paths in source and the current
commit, and private coordination links throughout the inspected metadata.
Private configured markers apply to all inspected files and local-ref metadata.
It reports file/line/rule only, withholding matched values and redacting a
filename that itself contains private markers. External or unresolved source
symlinks are refused. Portable example
accounts and standard operating-system paths are allowed. Files over 32 MiB
and submodule directories require separate review.

For deployment-specific names, addresses, machine labels and directory names,
set `BLUEFIN_PRIVATE_MARKERS_FILE` to a UTF-8 file **outside** the checkout.
Use one literal marker per line, at least four bytes. Matching ignores ASCII
case. Blank lines and lines beginning with `#` are ignored. Do not commit that
file or copy its contents into CI logs, fixtures or documentation.

CI also compiles `cli-tests.rs` and runs it with the checker executable and a
fresh external output directory. It exercises real disposable Git histories:
an older generic home example is allowed, a current private home is rejected,
and older private markers or coordination links remain fatal when the same
line contains a home path. Diagnostics must still withhold the private values.
It also verifies that inherited Git routing cannot select a different checkout,
and refuses a subdirectory scan that would omit sibling source files. Both the
checker and disposable Git fixtures ignore inherited `GIT_*` variables.

CI runs the generic check. Run the deployment-specific check before staging,
then repeat on a clean final commit. This check examines the working tree, not
alternate staged content or every historical file blob. Unpushed history,
public repository history and final app/package resources need their own
review. A passing marker scan cannot establish the absence of every possible
identity or secret. Keep private reports, device screenshots, signing material,
machine topology, media and account data outside source and release bundles.
