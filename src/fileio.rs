//! Loading and saving documents without document-sized temporaries.
//!
//! Load: the file is memory-mapped (its pages are file-backed, not private
//! memory), decoded in bounded chunks, and streamed into RichEdit with
//! EM_STREAMIN. Save: text is read back from RichEdit in chunks, encoded,
//! and written through one buffered writer.

use std::ffi::c_void;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::os::windows::io::AsRawHandle;
use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_READ, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, PAGE_READONLY,
    UnmapViewOfFile,
};
use windows::Win32::UI::Controls::RichEdit::{EDITSTREAM, EM_STREAMIN, SF_TEXT, SF_UNICODE};
use windows::core::PCWSTR;

use crate::app::{get_range, sm, text_len};
use crate::encoding::{self, Decoder, Encoding, Eol};

/// Read-only view of a whole file. Falls back to a heap copy for files the
/// system can't map (some network and virtual files).
enum Body {
    Mapped {
        view: MEMORY_MAPPED_VIEW_ADDRESS,
        mapping: HANDLE,
        len: usize,
    },
    Owned(Vec<u8>),
}

impl Body {
    fn open(path: &Path) -> std::io::Result<Body> {
        let file = File::open(path)?;
        let len = file.metadata()?.len() as usize;
        if len == 0 {
            return Ok(Body::Owned(Vec::new()));
        }
        unsafe {
            let handle = HANDLE(file.as_raw_handle() as *mut c_void);
            if let Ok(mapping) = CreateFileMappingW(handle, None, PAGE_READONLY, 0, 0, PCWSTR::null())
            {
                let view = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0);
                if !view.Value.is_null() {
                    return Ok(Body::Mapped { view, mapping, len });
                }
                let _ = CloseHandle(mapping);
            }
        }
        Ok(Body::Owned(std::fs::read(path)?))
    }

    fn bytes(&self) -> &[u8] {
        match self {
            Body::Mapped { view, len, .. } => unsafe {
                std::slice::from_raw_parts(view.Value as *const u8, *len)
            },
            Body::Owned(v) => v,
        }
    }
}

impl Drop for Body {
    fn drop(&mut self) {
        if let Body::Mapped { view, mapping, .. } = self {
            unsafe {
                let _ = UnmapViewOfFile(*view);
                let _ = CloseHandle(*mapping);
            }
        }
    }
}

/// EM_STREAMIN state: decoded units waiting to be handed to RichEdit.
struct InStream<'a> {
    dec: Decoder<'a>,
    buf: Vec<u16>,
    at: usize,
}

unsafe extern "system" fn stream_in(cookie: usize, out: *mut u8, cb: i32, pcb: *mut i32) -> u32 {
    unsafe {
        let s = &mut *(cookie as *mut InStream);
        let room = (cb.max(0) as usize) / 2;
        let mut written = 0usize;
        while written < room {
            if s.at == s.buf.len() {
                s.at = 0;
                if !s.dec.next_chunk(&mut s.buf) {
                    break;
                }
                continue; // a chunk can normalize down to nothing
            }
            let n = (s.buf.len() - s.at).min(room - written);
            std::ptr::copy_nonoverlapping(
                s.buf.as_ptr().add(s.at) as *const u8,
                out.add(written * 2),
                n * 2,
            );
            s.at += n;
            written += n;
        }
        *pcb = (written * 2) as i32;
        0
    }
}

/// Replaces the editor's contents with the file at `path`. Text is decoded
/// in bounded chunks and streamed in, so the only full-size copy is
/// RichEdit's own.
pub fn load(edit: HWND, path: &Path) -> std::io::Result<(Encoding, Eol)> {
    let body = Body::open(path)?;
    let bytes = body.bytes();
    let (enc, skip) = encoding::detect(bytes);
    let text = &bytes[skip..];
    let eol = encoding::detect_eol(text, enc);
    let mut st = InStream {
        dec: Decoder::new(text, enc),
        buf: Vec::new(),
        at: 0,
    };
    let es = EDITSTREAM {
        dwCookie: &mut st as *mut InStream as usize,
        dwError: 0,
        pfnCallback: Some(stream_in),
    };
    sm(edit, EM_STREAMIN, (SF_TEXT | SF_UNICODE) as usize, &es as *const EDITSTREAM as isize);
    let err = es.dwError;
    if err != 0 {
        return Err(std::io::Error::other(format!("stream error {err}")));
    }
    Ok((enc, eol))
}

/// Writes the editor's contents to `path` in `enc`, with `eol` breaks.
pub fn save(edit: HWND, path: &Path, enc: Encoding, eol: Eol) -> std::io::Result<()> {
    const CHUNK: i32 = 128 * 1024;
    let mut w = BufWriter::with_capacity(256 * 1024, File::create(path)?);
    w.write_all(encoding::bom(enc))?;
    let len = text_len(edit);
    let (mut scratch, mut out) = (Vec::new(), Vec::new());
    let mut pos = 0;
    while pos < len {
        let mut units = get_range(edit, pos, (pos + CHUNK).min(len));
        if units.is_empty() {
            break;
        }
        // Keep surrogate pairs whole across chunk boundaries.
        if pos + (units.len() as i32) < len && matches!(units.last(), Some(&(0xD800..=0xDBFF))) {
            units.pop();
        }
        pos += units.len() as i32;
        out.clear();
        encoding::encode_chunk(&units, enc, eol, &mut scratch, &mut out);
        w.write_all(&out)?;
    }
    w.flush()
}
