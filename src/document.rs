use std::path::PathBuf;
use windows::Win32::Foundation::HWND;

use crate::encoding::Encoding;

/// One open document: a file path (or untitled) plus its editor control.
/// Each tab owns a dedicated RichEdit so switching tabs preserves undo
/// history, scroll position, and selection for free.
pub struct Document {
    pub edit: HWND,
    pub path: Option<PathBuf>,
    pub encoding: Encoding,
    pub zoom: i32,
    /// Start offset of every logical line. RichEdit's own line messages
    /// count wrapped display rows, so line numbers, Ln/Col, and Go To use
    /// this instead. Rebuilt lazily after edits.
    pub line_starts: Vec<i32>,
    pub lines_dirty: bool,
}

impl Document {
    pub fn new(edit: HWND, path: Option<PathBuf>) -> Self {
        Self {
            edit,
            path,
            encoding: Encoding::Utf8,
            zoom: 100,
            line_starts: vec![0],
            lines_dirty: true,
        }
    }

    pub fn name(&self) -> String {
        self.path
            .as_deref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".to_string())
    }
}
