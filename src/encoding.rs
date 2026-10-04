//! Text encodings and line endings. Everything works on bounded chunks so a
//! file of any size passes through without a document-sized copy.

use windows::Win32::Globalization::{
    CP_ACP, CP_UTF8, CPINFO, GetCPInfo, MultiByteToWideChar, WideCharToMultiByte,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Ansi,
}

impl Encoding {
    pub fn label(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf8Bom => "UTF-8 BOM",
            Encoding::Utf16Le => "UTF-16 LE",
            Encoding::Utf16Be => "UTF-16 BE",
            Encoding::Ansi => "ANSI",
        }
    }

    fn bom(self) -> &'static [u8] {
        match self {
            Encoding::Utf8Bom => &[0xEF, 0xBB, 0xBF],
            Encoding::Utf16Le => &[0xFF, 0xFE],
            Encoding::Utf16Be => &[0xFE, 0xFF],
            _ => &[],
        }
    }
}

/// Line-ending style, detected on load and written back on save.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Eol {
    Crlf,
    Lf,
    Cr,
}

impl Eol {
    pub fn label(self) -> &'static str {
        match self {
            Eol::Crlf => "Windows (CRLF)",
            Eol::Lf => "Unix (LF)",
            Eol::Cr => "Macintosh (CR)",
        }
    }
}

/// Picks the encoding from the BOM or content. Returns the encoding and
/// how many leading bytes (the BOM) to skip.
pub fn detect(bytes: &[u8]) -> (Encoding, usize) {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return (Encoding::Utf16Le, 2);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return (Encoding::Utf16Be, 2);
    }
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return (Encoding::Utf8Bom, 3);
    }
    if looks_utf16(bytes, 1) {
        return (Encoding::Utf16Le, 0);
    }
    if looks_utf16(bytes, 0) {
        return (Encoding::Utf16Be, 0);
    }
    // Validation scans without allocating.
    match std::str::from_utf8(bytes) {
        Ok(_) => (Encoding::Utf8, 0),
        Err(_) => (Encoding::Ansi, 0),
    }
}

/// Heuristic: BOM-less UTF-16 has a NUL in one byte of most pairs.
/// `zero_at` is 1 for little-endian (high byte), 0 for big-endian.
fn looks_utf16(bytes: &[u8], zero_at: usize) -> bool {
    let pairs = bytes.len().min(512) / 2;
    if pairs < 2 {
        return false;
    }
    let zeros = (0..pairs).filter(|&i| bytes[2 * i + zero_at] == 0).count();
    zeros * 10 > pairs * 7
}

/// The first line break decides the style; none at all means CRLF.
pub fn detect_eol(body: &[u8], enc: Encoding) -> Eol {
    const PROBE: usize = 1 << 20;
    let unit = |i: usize| -> Option<u16> {
        match enc {
            Encoding::Utf16Le => body.get(2 * i..2 * i + 2).map(|b| u16::from_le_bytes([b[0], b[1]])),
            Encoding::Utf16Be => body.get(2 * i..2 * i + 2).map(|b| u16::from_be_bytes([b[0], b[1]])),
            _ => body.get(i).map(|&b| b as u16),
        }
    };
    let n = match enc {
        Encoding::Utf16Le | Encoding::Utf16Be => body.len() / 2,
        _ => body.len(),
    }
    .min(PROBE);
    for i in 0..n {
        match unit(i) {
            Some(10) => return Eol::Lf,
            Some(13) => return if unit(i + 1) == Some(10) { Eol::Crlf } else { Eol::Cr },
            _ => {}
        }
    }
    Eol::Crlf
}

/// True when the ANSI code page is single-byte, so it can be decoded in
/// arbitrary chunks. DBCS code pages decode in one piece instead.
fn ansi_is_sbcs() -> bool {
    let mut info = CPINFO::default();
    unsafe { GetCPInfo(CP_ACP, &mut info).is_ok() && info.MaxCharSize == 1 }
}

/// Decodes a byte body chunk by chunk into UTF-16, turning every line
/// break (CRLF, LF, or CR) into a lone CR — RichEdit's own paragraph mark.
pub struct Decoder<'a> {
    body: &'a [u8],
    pos: usize,
    enc: Encoding,
    sbcs: bool,
    prev_cr: bool,
}

/// Source bytes decoded per step (bounded working set).
const CHUNK: usize = 256 * 1024;

impl<'a> Decoder<'a> {
    pub fn new(body: &'a [u8], enc: Encoding) -> Self {
        Self {
            body,
            pos: 0,
            enc,
            sbcs: enc != Encoding::Ansi || ansi_is_sbcs(),
            prev_cr: false,
        }
    }

    /// Decodes the next chunk into `out` (cleared first). Returns false at
    /// the end of the input.
    pub fn next_chunk(&mut self, out: &mut Vec<u16>) -> bool {
        out.clear();
        let rest = &self.body[self.pos..];
        if rest.is_empty() {
            return false;
        }
        match self.enc {
            Encoding::Utf16Le | Encoding::Utf16Be => {
                let take = rest.len().min(CHUNK) & !1;
                let take = if take == 0 { rest.len() } else { take };
                let le = self.enc == Encoding::Utf16Le;
                out.extend(rest[..take].chunks_exact(2).map(|c| {
                    if le {
                        u16::from_le_bytes([c[0], c[1]])
                    } else {
                        u16::from_be_bytes([c[0], c[1]])
                    }
                }));
                self.pos += take;
            }
            Encoding::Utf8 | Encoding::Utf8Bom | Encoding::Ansi => {
                let mut take = if self.sbcs { rest.len().min(CHUNK) } else { rest.len() };
                if self.enc != Encoding::Ansi {
                    // Never split a UTF-8 sequence: back up to a lead byte.
                    while take < rest.len() && take > 0 && (rest[take] & 0xC0) == 0x80 {
                        take -= 1;
                    }
                }
                let cp = if self.enc == Encoding::Ansi { CP_ACP } else { CP_UTF8 };
                out.resize(take, 0); // UTF-16 never needs more units than bytes
                let n = unsafe {
                    MultiByteToWideChar(cp, Default::default(), &rest[..take], Some(out))
                };
                out.truncate(n.max(0) as usize);
                self.pos += take;
            }
        }
        self.normalize(out);
        true
    }

    fn normalize(&mut self, out: &mut Vec<u16>) {
        let mut w = 0;
        for r in 0..out.len() {
            let c = out[r];
            if c == 10 {
                if self.prev_cr {
                    self.prev_cr = false;
                    continue; // LF of a CRLF pair
                }
                out[w] = 13;
            } else {
                self.prev_cr = c == 13;
                out[w] = c;
            }
            w += 1;
        }
        out.truncate(w);
    }
}

/// Appends `units` (RichEdit text: CR line breaks) to `out` in `enc`,
/// expanding each CR to `eol`. `units` must not end inside a surrogate pair.
pub fn encode_chunk(units: &[u16], enc: Encoding, eol: Eol, scratch: &mut Vec<u16>, out: &mut Vec<u8>) {
    let text: &[u16] = if eol == Eol::Cr {
        units
    } else {
        scratch.clear();
        for &c in units {
            if c == 13 {
                if eol == Eol::Crlf {
                    scratch.push(13);
                }
                scratch.push(10);
            } else {
                scratch.push(c);
            }
        }
        scratch
    };
    match enc {
        Encoding::Utf16Le => text.iter().for_each(|u| out.extend_from_slice(&u.to_le_bytes())),
        Encoding::Utf16Be => text.iter().for_each(|u| out.extend_from_slice(&u.to_be_bytes())),
        Encoding::Utf8 | Encoding::Utf8Bom | Encoding::Ansi => {
            if text.is_empty() {
                return;
            }
            let cp = if enc == Encoding::Ansi { CP_ACP } else { CP_UTF8 };
            let start = out.len();
            out.resize(start + text.len() * 3, 0); // UTF-8 worst case per unit
            let n = unsafe { WideCharToMultiByte(cp, 0, text, Some(&mut out[start..]), None, None) };
            out.truncate(start + n.max(0) as usize);
        }
    }
}

/// Byte-order mark to write before the body.
pub fn bom(enc: Encoding) -> &'static [u8] {
    enc.bom()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_all(body: &[u8], enc: Encoding) -> Vec<u16> {
        let mut d = Decoder::new(body, enc);
        let (mut out, mut chunk) = (Vec::new(), Vec::new());
        while d.next_chunk(&mut chunk) {
            out.extend_from_slice(&chunk);
        }
        out
    }

    fn encode_all(units: &[u16], enc: Encoding, eol: Eol) -> Vec<u8> {
        let (mut scratch, mut out) = (Vec::new(), Vec::new());
        out.extend_from_slice(bom(enc));
        encode_chunk(units, enc, eol, &mut scratch, &mut out);
        out
    }

    #[test]
    fn line_breaks_normalize_to_cr() {
        let want: Vec<u16> = "a\rb\rc\r\rd".encode_utf16().collect();
        for src in ["a\r\nb\r\nc\r\n\r\nd", "a\nb\nc\n\nd", "a\rb\rc\r\rd", "a\r\nb\nc\r\rd"] {
            assert_eq!(decode_all(src.as_bytes(), Encoding::Utf8), want, "{src:?}");
        }
    }

    #[test]
    fn crlf_split_across_chunks_is_one_break() {
        // Put a CRLF exactly on the chunk boundary.
        let mut s = "x".repeat(CHUNK - 1);
        s.push_str("\r\nyz");
        let out = decode_all(s.as_bytes(), Encoding::Utf8);
        assert_eq!(out.len(), CHUNK - 1 + 1 + 2);
        assert_eq!(out[CHUNK - 1], 13);
        assert_eq!(&out[CHUNK..], &[b'y' as u16, b'z' as u16]);
    }

    #[test]
    fn utf8_sequences_survive_chunk_boundaries() {
        // 3-byte characters straddling every possible boundary offset.
        let mut s = "a".repeat(CHUNK - 2);
        s.push_str(&"€".repeat(10));
        let out = decode_all(s.as_bytes(), Encoding::Utf8);
        assert_eq!(String::from_utf16(&out).unwrap(), s);
    }

    #[test]
    fn detects_encodings_and_eols() {
        assert!(detect(&[0xEF, 0xBB, 0xBF, b'a']) == (Encoding::Utf8Bom, 3));
        assert!(detect(&[0xFF, 0xFE, b'a', 0]) == (Encoding::Utf16Le, 2));
        assert!(detect(&[0xFE, 0xFF, 0, b'a']) == (Encoding::Utf16Be, 2));
        assert!(detect("héllo".as_bytes()) == (Encoding::Utf8, 0));
        assert!(detect(&[b'h', 0xE9, b'l']) == (Encoding::Ansi, 0));
        assert!(detect_eol(b"a\r\nb", Encoding::Utf8) == Eol::Crlf);
        assert!(detect_eol(b"a\nb\r\n", Encoding::Utf8) == Eol::Lf);
        assert!(detect_eol(b"a\rb", Encoding::Utf8) == Eol::Cr);
        assert!(detect_eol(b"ab", Encoding::Utf8) == Eol::Crlf);
        assert!(detect_eol(&[b'a', 0, b'\n', 0], Encoding::Utf16Le) == Eol::Lf);
    }

    #[test]
    fn round_trips_every_encoding_and_eol() {
        let text = "line one\r\nzwei — ünïcödé 😀\r\n\r\nlast";
        for enc in [Encoding::Utf8, Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be] {
            for (eol, nl) in [(Eol::Crlf, "\r\n"), (Eol::Lf, "\n"), (Eol::Cr, "\r")] {
                let original = text.replace("\r\n", nl);
                let bytes = match enc {
                    Encoding::Utf16Le => encode_all(&original.encode_utf16().collect::<Vec<_>>(), enc, Eol::Cr),
                    _ => original.clone().into_bytes(),
                };
                // Decode what's on disk, then write it back in the same style.
                let (e, skip) = detect(&bytes);
                let units = decode_all(&bytes[skip..], e);
                let again = encode_all(&units, e, detect_eol(&bytes[skip..], e));
                if enc == Encoding::Utf16Le || enc == Encoding::Utf8 {
                    assert_eq!(again, bytes, "{:?}", (enc.label(), eol.label()));
                }
                // And the text itself survives any encoding.
                let back = decode_all(&encode_all(&units, enc, eol)[bom(enc).len()..], enc);
                assert_eq!(back, units);
            }
        }
    }
}
