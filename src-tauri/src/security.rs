// Root-anchored security boundary. `canonicalize_and_check_root` is the
// load-bearing gate every filesystem read in the IPC layer flows through.
//
// The gate itself lives in `vlerv-remote` — the remote stack has to run it on
// every path a peer can reach, and a headless host must get the identical
// check. It is re-exported here so the app's own callers keep one name for it:
// there is exactly ONE gate on this machine, not an app copy and a remote copy.

pub use vlerv_remote::security::{
    canonicalize_allow_external, canonicalize_allow_rootless, canonicalize_and_check_root,
    OutOfRootError, RootSet,
};
