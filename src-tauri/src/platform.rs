// Which OS the Rust core was compiled for. The frontend is one React bundle
// shared by the macOS app and the iOS companion, so it needs a truthful
// answer before it renders desktop-only chrome (drag-out, share sheet,
// Reveal in Finder, window-state) or calls a macOS-only command.
//
// Compile-time, not a user-agent sniff: the value comes from the same `cfg`
// that decides which commands exist.

/// `{ "os": "macos" | "ios" }` — the shape `invoke("platform_info")` returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlatformInfo {
    pub os: &'static str,
}

/// The OS this binary was built for. `std::env::consts::OS` already reports
/// `"macos"` / `"ios"`, so there is no table to keep in sync.
#[tauri::command]
pub fn platform_info() -> PlatformInfo {
    PlatformInfo { os: std::env::consts::OS }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_the_compiled_target_os() {
        // The two platforms PRODUCT.md commits to. The assertion is the
        // contract the frontend switches on.
        assert!(
            matches!(platform_info().os, "macos" | "ios"),
            "unexpected target os: {}",
            platform_info().os
        );
        #[cfg(target_os = "macos")]
        assert_eq!(platform_info().os, "macos");
        #[cfg(target_os = "ios")]
        assert_eq!(platform_info().os, "ios");
    }

    #[test]
    fn serializes_as_a_bare_os_field() {
        let json = serde_json::to_value(platform_info()).expect("serialize");
        assert_eq!(json, serde_json::json!({ "os": platform_info().os }));
    }
}
