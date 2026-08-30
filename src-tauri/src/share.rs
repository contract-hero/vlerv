// macOS share sheet via NSSharingServicePicker. The picker is anchored at a
// webview-relative client rect so the popover opens from the Share button.
//
// Two things reach it: files (Preview's Share button) and links (the
// `vlerv://` pairing / beam tickets). They share the picker and the anchor;
// only the pasteboard item and the admission check differ.

use crate::deeplink::DeepLinkIntent;
use crate::security::{self, RootSet};

/// Admit a link to the share sheet.
///
/// `deeplink::parse` owns the question "is this a link a Vlervtifacts can
/// open", so this defers to it instead of growing a second, looser rule that
/// is free to drift: a link the recipient's app would refuse must never leave
/// this machine. Parsing also brings the ticket charset check along, which
/// rejects whitespace, NUL and quoting tricks inside the ticket.
///
/// Only the two ticket verbs are shareable. `open`, `reveal` and `beam` carry
/// a local filesystem path, which is this machine's business and nobody
/// else's — sending one would leak a path and arrive as a dead link.
///
/// Pure, so it is unit-testable without AppKit.
fn validate_link(link: &str) -> Result<&str, String> {
    let trimmed = link.trim();
    match crate::deeplink::parse(trimmed) {
        Ok(DeepLinkIntent::Pair { .. } | DeepLinkIntent::Receive { .. }) => Ok(trimmed),
        _ => Err("link is not shareable".into()),
    }
}

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

/// Canonicalize share candidates through the shared external-file policy
/// (`security::canonicalize_allow_external`): out-of-root files that resolve
/// are shareable on purpose — Preview legitimately displays them (ad-hoc
/// opens, out-of-root deep links). Anything the OS can't resolve — or an
/// empty root set — is rejected. Factored out of the command so the
/// non-AppKit half is unit-testable.
fn resolve_paths(paths: &[String], roots: &RootSet) -> Result<Vec<String>, String> {
    if paths.is_empty() {
        return Err("nothing to share".into());
    }
    paths
        .iter()
        .map(|p| {
            security::canonicalize_allow_external(std::path::Path::new(p), roots)
                .map(|(canonical, _out_of_root)| canonical.to_string_lossy().into_owned())
                // Same no-existence-leak wording as the deep-link reveal handler.
                .map_err(|_| "path not found or out of root".to_string())
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

    /// Pop a picker for `items` at `anchor`. The half both entry points
    /// below share; only the pasteboard items differ.
    ///
    /// # Safety
    /// Must be called on the main thread with a valid WKWebView pointer
    /// (WKWebView is an NSView subclass). NSSharingServicePicker is not
    /// marked MainThreadOnly in objc2-app-kit, so the compiler cannot
    /// enforce the threading requirement — the caller must.
    unsafe fn present(
        ns_view: *mut std::ffi::c_void,
        items: &[Retained<AnyObject>],
        anchor: AnchorRect,
    ) {
        let view: &NSView = &*(ns_view as *const NSView);
        let items: Retained<NSArray<AnyObject>> = NSArray::from_retained_slice(items);

        let picker =
            NSSharingServicePicker::initWithItems(NSSharingServicePicker::alloc(), &items);

        // CSS client coords are top-left-origin. WKWebView overrides
        // isFlipped to YES, so they map 1:1 (CSS px == AppKit points) with
        // no y-flip. The assert guards the invariant if the anchor view
        // ever changes.
        debug_assert!(view.isFlipped(), "share anchor view must be flipped (WKWebView)");
        let rect = NSRect::new(
            NSPoint::new(anchor.x, anchor.y),
            NSSize::new(anchor.width.max(1.0), anchor.height.max(1.0)),
        );

        // NSMinYEdge: AppKit applies the edge after converting to screen
        // space (y-up), so the popover opens from the button's visual bottom.
        picker.showRelativeToRect_ofView_preferredEdge(rect, view, NSRectEdge::NSMinYEdge);
        LIVE_PICKER.with(|slot| *slot.borrow_mut() = Some(picker));
    }

    /// # Safety
    /// Same contract as `present`.
    pub unsafe fn show_file_picker(
        ns_view: *mut std::ffi::c_void,
        paths: &[String],
        anchor: AnchorRect,
    ) {
        let urls: Vec<Retained<AnyObject>> = paths
            .iter()
            .map(|p| {
                // Plain filesystem path, never a file:// string — NSURL
                // conforms to NSPasteboardWriting, which initWithItems needs.
                let url = NSURL::fileURLWithPath(&NSString::from_str(p));
                Retained::into_super(Retained::into_super(url))
            })
            .collect();
        present(ns_view, &urls, anchor);
    }

    /// A link, not a file. NSURL is the item AirDrop and Messages want — the
    /// recipient gets something tappable that the `vlerv://` handler opens.
    /// If NSURL will not parse the string, the NSString goes on the
    /// pasteboard instead: a link the user can still paste beats a share
    /// sheet that opens with nothing in it.
    ///
    /// # Safety
    /// Same contract as `present`.
    pub unsafe fn show_link_picker(
        ns_view: *mut std::ffi::c_void,
        link: &str,
        anchor: AnchorRect,
    ) {
        let text = NSString::from_str(link);
        let item: Retained<AnyObject> = match NSURL::URLWithString(&text) {
            Some(url) => Retained::into_super(Retained::into_super(url)),
            None => Retained::into_super(Retained::into_super(text)),
        };
        present(ns_view, &[item], anchor);
    }
}

/// Share one or more files via the native macOS share sheet, anchored at a
/// webview-relative client rect. Fire-and-forget: the picker's outcome
/// (service chosen / dismissed) is not reported back.
///
/// `async` so the per-path canonicalize syscalls run on the runtime pool
/// instead of the main thread (sync commands execute on the main thread);
/// only the picker needs the main thread, and `with_webview` hops there.
#[tauri::command]
pub async fn share_file(
    webview: tauri::WebviewWindow,
    roots: tauri::State<'_, RootSet>,
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
                macos::show_file_picker(wv.inner() as *mut std::ffi::c_void, &resolved, anchor);
            })
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Total invoke surface: the command exists on every platform and
        // answers with a clean error instead of a missing-command failure.
        // The iOS companion is read-only and has no NSSharingServicePicker;
        // a UIActivityViewController equivalent is a separate decision.
        let _ = (webview, anchor, resolved);
        Err("share sheet is not available on this platform".into())
    }
}

/// Share a `vlerv://` link (a pairing or beam ticket) via the native macOS
/// share sheet, anchored like `share_file`. AirDrop, Messages and Mail all
/// take a URL, which is the whole point: the link has to reach the other
/// device before it can be opened there.
///
/// Fire-and-forget, same as `share_file`: the picker's outcome is not
/// reported back. iOS has no NSSharingServicePicker and this crate carries
/// no UIKit bindings, so the companion uses the WKWebView Web Share API from
/// the frontend and never calls this command.
#[tauri::command]
pub async fn share_link(
    webview: tauri::WebviewWindow,
    link: String,
    anchor: AnchorRect,
) -> Result<(), String> {
    let validated = validate_link(&link)?.to_string();
    #[cfg(target_os = "macos")]
    {
        webview
            .with_webview(move |wv| unsafe {
                macos::show_link_picker(wv.inner() as *mut std::ffi::c_void, &validated, anchor);
            })
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (webview, anchor, validated);
        Err("share sheet is not available on this platform".into())
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_paths, validate_link};
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
        let resolved =
            resolve_paths(&[other_file.to_string_lossy().into_owned()], &roots).expect("external");
        assert_eq!(
            resolved,
            vec![other_file.canonicalize().unwrap().to_string_lossy().into_owned()]
        );
    }

    #[test]
    fn one_bad_path_rejects_the_whole_batch() {
        // Fail-closed: a share is all-or-nothing, never a silent subset.
        let (dir, file) = root_and_file();
        let roots = RootSet::new(vec![dir.path().to_path_buf()]);
        let paths = [
            file.to_string_lossy().into_owned(),
            "/definitely/not/a/real/path".to_string(),
        ];
        assert_eq!(
            resolve_paths(&paths, &roots).unwrap_err(),
            "path not found or out of root"
        );
    }

    #[test]
    fn resolves_multiple_paths_in_input_order() {
        let (dir, a) = root_and_file();
        let roots = RootSet::new(vec![dir.path().to_path_buf()]);
        let b = dir.path().join("second.txt");
        std::fs::write(&b, "y").expect("write");
        let resolved = resolve_paths(
            &[a.to_string_lossy().into_owned(), b.to_string_lossy().into_owned()],
            &roots,
        )
        .expect("both in-root");
        assert_eq!(resolved, vec![
            a.canonicalize().unwrap().to_string_lossy().into_owned(),
            b.canonicalize().unwrap().to_string_lossy().into_owned(),
        ]);
    }

    #[test]
    fn accepts_the_two_link_shapes_the_ui_mints() {
        // Exactly what peers::build_pair_link and beam::build_link produce.
        let pair = "vlerv://pair?ticket=ABC";
        let receive = "vlerv://receive?ticket=ABC&name=report.html&size=12";
        assert_eq!(validate_link(pair).unwrap(), pair);
        assert_eq!(validate_link(receive).unwrap(), receive);
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(validate_link("  vlerv://pair?ticket=ABC\n").unwrap(), "vlerv://pair?ticket=ABC");
    }

    #[test]
    fn rejects_every_other_scheme() {
        for link in ["file:///etc/passwd", "javascript:alert(1)", "https://example.com", "pair?ticket=A"] {
            assert!(validate_link(link).is_err(), "{link} must not reach the share sheet");
        }
    }

    #[test]
    fn rejects_the_verbs_that_carry_a_local_path() {
        // Parseable, but they name a file on THIS machine — sharing one leaks
        // a path and lands as a dead link.
        for link in ["vlerv://open?path=/etc/passwd", "vlerv://reveal?path=/etc", "vlerv://beam?path=/etc/passwd"] {
            assert!(validate_link(link).is_err(), "{link} must not reach the share sheet");
        }
    }

    #[test]
    fn rejects_a_bare_scheme_and_a_ticketless_verb() {
        assert!(validate_link("vlerv://").is_err());
        assert!(validate_link("vlerv://pair").is_err());
        assert!(validate_link("vlerv://receive?t=ABC").is_err());
    }

    #[test]
    fn rejects_whitespace_and_control_characters_inside_a_ticket() {
        // A link that cannot survive a message body is worse than no link;
        // deeplink's alphanumeric ticket rule is what catches these.
        assert!(validate_link("vlerv://pair?ticket=A B").is_err());
        assert!(validate_link("vlerv://pair?ticket=A\nB").is_err());
    }
}
