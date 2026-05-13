// Drag-spike payload builder — informational probe. Returns the
// pasteboard-payload shape (UTType + file URL) the spike handler dispatches.

use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DragPayload {
    pub ut_type: String,
    pub file_url: String,
}

const FILE_PATH: &AsciiSet = &CONTROLS
    .add(b' ').add(b'"').add(b'<').add(b'>').add(b'`')
    .add(b'#').add(b'?').add(b'{').add(b'}').add(b'^').add(b'\\');

pub fn build_payload(path: &Path) -> DragPayload {
    let path_str = path.to_string_lossy();
    let encoded: String = utf8_percent_encode(&path_str, FILE_PATH).collect();
    DragPayload {
        ut_type: "public.file-url".to_string(),
        file_url: format!("file://{encoded}"),
    }
}
