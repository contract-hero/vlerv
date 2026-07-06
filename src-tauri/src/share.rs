// macOS share sheet via NSSharingServicePicker. The picker is anchored at a
// webview-relative client rect so the popover opens from the Share button.

use crate::security::{self, RootSet};

/// Anchor rect in webview client coordinates, straight from
/// `getBoundingClientRect()`. WKWebView is a flipped NSView, so CSS px map
/// 1:1 to AppKit points with no y-flip and no devicePixelRatio scaling.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
pub struct AnchorRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Canonicalize share candidates through the root gate before touching
/// AppKit. Out-of-root files that resolve are shareable on purpose: Preview
/// legitimately displays them (ad-hoc opens, out-of-root deep links), so this
/// mirrors `dispatch_deep_link`'s `OutOfRoot(canonical)` branch. Anything the
/// OS can't resolve — or an empty root set — is rejected. Factored out of the
/// command so the non-AppKit half is unit-testable.
fn resolve_paths(paths: &[String], roots: &RootSet) -> Result<Vec<String>, String> {
    if paths.is_empty() {
        return Err("nothing to share".into());
    }
    paths
        .iter()
        .map(|p| {
            match security::canonicalize_and_check_root(std::path::Path::new(p), roots) {
                Ok(canonical) | Err(security::OutOfRootError::OutOfRoot(canonical)) => {
                    Ok(canonical.to_string_lossy().into_owned())
                }
                // Same no-existence-leak wording as the deep-link reveal handler.
                Err(_) => Err("path not found or out of root".to_string()),
            }
        })
        .collect()
}

#[cfg(target_os = "macos")]
mod macos {
    use std::cell::RefCell;

    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::AnyThread;
    use objc2_app_kit::{NSSharingServicePicker, NSView};
    use objc2_foundation::{NSArray, NSPoint, NSRect, NSRectEdge, NSSize, NSString, NSURL};

    use super::AnchorRect;

    thread_local! {
        // Keep the picker alive while its popover is open: dropping the
        // Retained at end of scope can dealloc the picker mid-presentation
        // and crash on dismiss. Retain-until-next-share is the minimal fix;
        // a NSSharingServicePickerDelegate with self-retention would also
        // report shared/cancelled back to JS, which fire-and-forget skips.
        static LIVE_PICKER: RefCell<Option<Retained<NSSharingServicePicker>>> =
            const { RefCell::new(None) };
    }

    /// # Safety
    /// Must be called on the main thread with a valid WKWebView pointer
    /// (WKWebView is an NSView subclass). NSSharingServicePicker is not
    /// marked MainThreadOnly in objc2-app-kit, so the compiler cannot
    /// enforce the threading requirement — the caller must.
    pub unsafe fn show_picker(
        ns_view: *mut std::ffi::c_void,
        paths: &[String],
        anchor: AnchorRect,
    ) {
        let view: &NSView = &*(ns_view as *const NSView);

        let urls: Vec<Retained<AnyObject>> = paths
            .iter()
            .map(|p| {
                // Plain filesystem path, never a file:// string — NSURL
                // conforms to NSPasteboardWriting, which initWithItems needs.
                let url = NSURL::fileURLWithPath(&NSString::from_str(p));
                Retained::into_super(Retained::into_super(url))
            })
            .collect();
        let items: Retained<NSArray<AnyObject>> = NSArray::from_retained_slice(&urls);

        let picker =
            NSSharingServicePicker::initWithItems(NSSharingServicePicker::alloc(), &items);

        // CSS client coords are top-left-origin. WKWebView is a flipped
        // NSView, so they map 1:1 (CSS px == AppKit points). Defensive
        // conversion in case the anchor view is ever non-flipped.
        let bounds = view.bounds();
        let height = anchor.height.max(1.0);
        let anchor_y = if view.isFlipped() {
            anchor.y
        } else {
            bounds.size.height - anchor.y - height
        };
        let rect = NSRect::new(
            NSPoint::new(anchor.x, anchor_y),
            NSSize::new(anchor.width.max(1.0), height),
        );

        // NSMinYEdge: AppKit applies the edge after converting to screen
        // space (y-up), so the popover opens from the button's visual bottom.
        picker.showRelativeToRect_ofView_preferredEdge(rect, view, NSRectEdge::NSMinYEdge);
        LIVE_PICKER.with(|slot| *slot.borrow_mut() = Some(picker));
    }
}

/// Share one or more files via the native macOS share sheet, anchored at a
/// webview-relative client rect. Fire-and-forget: the picker's outcome
/// (service chosen / dismissed) is not reported back.
#[tauri::command]
pub fn share_file(
    webview: tauri::WebviewWindow,
    roots: tauri::State<RootSet>,
    paths: Vec<String>,
    anchor: AnchorRect,
) -> Result<(), String> {
    let resolved = resolve_paths(&paths, &roots)?;
    #[cfg(target_os = "macos")]
    {
        // with_webview's closure runs on the main thread (documented) and
        // hands over the WKWebView pointer — both things the picker needs.
        webview
            .with_webview(move |wv| unsafe {
                macos::show_picker(wv.inner() as *mut std::ffi::c_void, &resolved, anchor);
            })
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (webview, anchor, resolved);
        Err("share sheet is macOS-only".into())
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_paths;
    use crate::security::RootSet;

    fn root_and_file() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tmp dir");
        let file = dir.path().join("shared.txt");
        std::fs::write(&file, "x").expect("write");
        (dir, file)
    }

    #[test]
    fn rejects_empty_list() {
        let (dir, _file) = root_and_file();
        let roots = RootSet::new(vec![dir.path().to_path_buf()]);
        assert!(resolve_paths(&[], &roots).is_err());
    }

    #[test]
    fn rejects_missing_path_without_leaking_existence() {
        let (dir, _file) = root_and_file();
        let roots = RootSet::new(vec![dir.path().to_path_buf()]);
        let err = resolve_paths(&["/definitely/not/a/real/path".into()], &roots).unwrap_err();
        assert_eq!(err, "path not found or out of root");
    }

    #[test]
    fn rejects_when_root_set_is_empty() {
        let (_dir, file) = root_and_file();
        let err = resolve_paths(&[file.to_string_lossy().into_owned()], &RootSet::empty())
            .unwrap_err();
        assert_eq!(err, "path not found or out of root");
    }

    #[test]
    fn accepts_in_root_paths_canonicalized() {
        let (dir, file) = root_and_file();
        let roots = RootSet::new(vec![dir.path().to_path_buf()]);
        let resolved =
            resolve_paths(&[file.to_string_lossy().into_owned()], &roots).expect("in-root");
        // Canonical form (tmpdirs on macOS resolve /var -> /private/var).
        assert_eq!(resolved, vec![file.canonicalize().unwrap().to_string_lossy().into_owned()]);
    }

    #[test]
    fn accepts_out_of_root_existing_paths() {
        // Mirrors dispatch_deep_link: Preview displays ad-hoc external files,
        // so a resolvable out-of-root path is shareable.
        let (dir, _file) = root_and_file();
        let roots = RootSet::new(vec![dir.path().to_path_buf()]);
        let (other_dir, other_file) = root_and_file();
        let _keep = other_dir;
        assert!(resolve_paths(&[other_file.to_string_lossy().into_owned()], &roots).is_ok());
    }
}
