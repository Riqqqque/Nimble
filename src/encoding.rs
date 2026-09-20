use windows::Win32::Globalization::{CP_ACP, CP_UTF8, MultiByteToWideChar, WideCharToMultiByte};

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
}

/// Decode file bytes into UTF-16 code units, detecting the source encoding.
pub fn decode(bytes: &[u8]) -> (Vec<u16>, Encoding) {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return (utf16_units(&bytes[2..], true), Encoding::Utf16Le);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return (utf16_units(&bytes[2..], false), Encoding::Utf16Be);
    }
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return (utf8_to_utf16(&bytes[3..]), Encoding::Utf8Bom);
    }
    if looks_utf16le(bytes) {
        return (utf16_units(bytes, true), Encoding::Utf16Le);
    }
    if looks_utf16be(bytes) {
        return (utf16_units(bytes, false), Encoding::Utf16Be);
    }
    match std::str::from_utf8(bytes) {
        Ok(_) => (utf8_to_utf16(bytes), Encoding::Utf8),
        Err(_) => (ansi_to_utf16(bytes), Encoding::Ansi),
    }
}

/// Encode UTF-16 code units back to bytes in the given encoding.
pub fn encode(units: &[u16], enc: Encoding) -> Vec<u8> {
    match enc {
        Encoding::Utf8 => utf16_to_cp(units, CP_UTF8),
        Encoding::Utf8Bom => {
            let mut v = Vec::with_capacity(units.len() + 3);
            v.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
            v.extend_from_slice(&utf16_to_cp(units, CP_UTF8));
            v
        }
        Encoding::Utf16Le => {
            let mut v = Vec::with_capacity(units.len() * 2 + 2);
            v.extend_from_slice(&[0xFF, 0xFE]);
            for &u in units {
                v.extend_from_slice(&u.to_le_bytes());
            }
            v
        }
        Encoding::Utf16Be => {
            let mut v = Vec::with_capacity(units.len() * 2 + 2);
            v.extend_from_slice(&[0xFE, 0xFF]);
            for &u in units {
                v.extend_from_slice(&u.to_be_bytes());
            }
            v
        }
        Encoding::Ansi => utf16_to_cp(units, CP_ACP),
    }
}

fn utf16_units(bytes: &[u8], little_endian: bool) -> Vec<u16> {
    bytes
        .chunks_exact(2)
        .map(|c| {
            if little_endian {
                u16::from_le_bytes([c[0], c[1]])
            } else {
                u16::from_be_bytes([c[0], c[1]])
            }
        })
        .collect()
}

/// Heuristic: UTF-16LE without BOM has NULs in the high byte of most char pairs.
fn looks_utf16le(bytes: &[u8]) -> bool {
    let pairs = bytes.len().min(512) / 2;
    if pairs < 2 {
        return false;
    }
    let hi_zeros = (0..pairs).filter(|&i| bytes[2 * i + 1] == 0).count();
    hi_zeros * 10 > pairs * 7
}

fn looks_utf16be(bytes: &[u8]) -> bool {
    let pairs = bytes.len().min(512) / 2;
    if pairs < 2 {
        return false;
    }
    let lo_zeros = (0..pairs).filter(|&i| bytes[2 * i] == 0).count();
    lo_zeros * 10 > pairs * 7
}

fn utf8_to_utf16(bytes: &[u8]) -> Vec<u16> {
    String::from_utf8_lossy(bytes).encode_utf16().collect()
}

fn ansi_to_utf16(bytes: &[u8]) -> Vec<u16> {
    unsafe {
        let n = MultiByteToWideChar(CP_ACP, Default::default(), bytes, None);
        if n <= 0 {
            return Vec::new();
        }
        let mut buf = vec![0u16; n as usize];
        let written = MultiByteToWideChar(CP_ACP, Default::default(), bytes, Some(&mut buf));
        buf.truncate(written.max(0) as usize);
        buf
    }
}

fn utf16_to_cp(units: &[u16], cp: u32) -> Vec<u8> {
    unsafe {
        let n = WideCharToMultiByte(cp, 0, units, None, None, None);
        if n <= 0 {
            return Vec::new();
        }
        let mut buf = vec![0u8; n as usize];
        let written = WideCharToMultiByte(cp, 0, units, Some(&mut buf), None, None);
        buf.truncate(written.max(0) as usize);
        buf
    }
}
