use cfg_aliases::cfg_aliases;

fn main() {
    // Setup cfg aliases
    cfg_aliases! {
        apple_mobile: { any(target_os = "ios", target_os = "tvos", target_os = "visionos", all(target_os = "macos", feature = "portable-apple")) },
        // Convenience aliases
        wasm_browser: { all(target_family = "wasm", target_os = "unknown") },
        // Limited POSIX platforms (not wasm)
        posix_minimal: { target_os = "espidf" },
        // Platforms where the `netdev` crate is available, i.e. everything
        // except esp-idf and wasm-in-browser. Keep in sync with the `netdev`
        // dependency target gate in Cargo.toml.
        netdev: { not(any(target_os = "espidf", all(target_family = "wasm", target_os = "unknown"))) },
        // BSD-derived platforms that share the `AF_ROUTE` routing-socket code.
        bsd: { all(not(apple_mobile), any(target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", target_os = "macos")) },
    }
}
