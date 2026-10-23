//! Font metadata and the Regular/Bold selection used by the menu and deployment.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct FontFace {
    pub path: PathBuf,
    pub family: String,
    pub style: String,
    pub weight: u16,
    pub italic: bool,
    pub variable: bool,
}

impl FontFace {
    pub fn load(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?;
        if !ext.eq_ignore_ascii_case("ttf") && !ext.eq_ignore_ascii_case("otf") {
            return None;
        }
        let data = fs::read(path).ok()?;
        let mut face = parse_face(&data)?;
        face.path = path.to_path_buf();
        Some(face)
    }

    pub fn deployed_name(&self, bold: bool) -> String {
        let role = if bold { "bold" } else { "regular" };
        let ext = self
            .path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("ttf");
        format!("cs2fc-{role}.{}", ext.to_ascii_lowercase())
    }

    pub fn label(&self) -> String {
        format!("{} / {} ({})", self.family, self.style, self.weight)
    }
}

#[derive(Default)]
pub struct FontSelection {
    pub regular: Option<FontFace>,
    pub bold: Option<FontFace>,
}

impl FontSelection {
    /// A single face keeps the legacy behavior. Two-face mode accepts upright static fonts.
    pub fn validate(&self) -> bool {
        match (&self.regular, &self.bold) {
            (Some(regular), Some(bold)) => {
                regular.weight <= 500
                    && bold.weight >= 600
                    && !regular.italic
                    && !bold.italic
                    && !regular.variable
                    && !bold.variable
                    && regular.path != bold.path
            }
            (Some(_), None) => true,
            _ => false,
        }
    }

    pub fn from_files(files: &[String]) -> Option<Self> {
        if files.is_empty() || files.len() > 2 {
            return None;
        }
        let mut faces = files
            .iter()
            .map(|p| FontFace::load(Path::new(p)))
            .collect::<Option<Vec<_>>>()?;
        faces.sort_by_key(|f| f.weight);
        let mut faces = faces.into_iter();
        let selection = Self {
            regular: faces.next(),
            bold: faces.next(),
        };
        selection.validate().then_some(selection)
    }
}

fn be_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let s = bytes.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_be_bytes([s[0], s[1]]))
}

fn be_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let s = bytes.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn first_face_offset(data: &[u8]) -> Option<usize> {
    match be_u32(data, 0)? {
        0x7474_6366 if be_u32(data, 8)? > 0 => Some(be_u32(data, 12)? as usize),
        0x0001_0000 | 0x4f54_544f | 0x7472_7565 | 0x7479_7031 => Some(0),
        _ => None,
    }
}

fn table<'a>(data: &'a [u8], face: usize, tag: &[u8; 4]) -> Option<&'a [u8]> {
    let count = be_u16(data, face.checked_add(4)?)? as usize;
    for i in 0..count {
        let record = face.checked_add(12)?.checked_add(i.checked_mul(16)?)?;
        if data.get(record..record.checked_add(4)?)? == tag {
            let offset = be_u32(data, record.checked_add(8)?)? as usize;
            let length = be_u32(data, record.checked_add(12)?)? as usize;
            return data.get(offset..offset.checked_add(length)?);
        }
    }
    None
}

fn name_string(names: &[u8], id: u16) -> Option<String> {
    let count = be_u16(names, 2)? as usize;
    let strings = be_u16(names, 4)? as usize;
    // Prefer English Windows names for consistency with Fontconfig, then Unicode/Mac.
    for pass in 0..4 {
        for i in 0..count {
            let record = 6 + i * 12;
            let Some(platform) = be_u16(names, record) else {
                continue;
            };
            let encoding = be_u16(names, record + 2)?;
            let language = be_u16(names, record + 4)?;
            if be_u16(names, record + 6)? != id {
                continue;
            }
            let eligible = match pass {
                0 => platform == 3 && language == 0x0409 && matches!(encoding, 0 | 1 | 10),
                1 => platform == 3 && matches!(encoding, 0 | 1 | 10),
                2 => platform == 0,
                _ => platform == 1 && encoding == 0,
            };
            if !eligible {
                continue;
            }
            let length = be_u16(names, record + 8)? as usize;
            let start = strings.checked_add(be_u16(names, record + 10)? as usize)?;
            let Some(bytes) = names.get(start..start.checked_add(length)?) else {
                continue;
            };
            let decoded = if platform == 1 {
                // ASCII is the interoperable subset of MacRoman family names.
                bytes
                    .is_ascii()
                    .then(|| String::from_utf8_lossy(bytes).into_owned())
            } else if bytes.len() % 2 == 0 {
                let units: Vec<_> = bytes
                    .chunks_exact(2)
                    .map(|c| u16::from_be_bytes([c[0], c[1]]))
                    .collect();
                String::from_utf16(&units).ok()
            } else {
                None
            };
            if let Some(s) = decoded {
                let s = s.trim();
                if !s.is_empty() && !s.chars().any(char::is_control) {
                    return Some(s.to_string());
                }
            }
        }
    }
    None
}

fn parse_face(data: &[u8]) -> Option<FontFace> {
    let face = first_face_offset(data)?;
    let names = table(data, face, b"name")?;
    let family = name_string(names, 16).or_else(|| name_string(names, 1))?;
    let style = name_string(names, 17)
        .or_else(|| name_string(names, 2))
        .unwrap_or_else(|| "Regular".into());
    let os2 = table(data, face, b"OS/2");
    let weight = os2
        .and_then(|t| be_u16(t, 4))
        .filter(|w| (1..=1000).contains(w))
        .unwrap_or_else(|| {
            if style.to_ascii_lowercase().contains("bold") {
                700
            } else {
                400
            }
        });
    let selection = os2.and_then(|t| be_u16(t, 62)).unwrap_or(0);
    let mac_style = table(data, face, b"head")
        .and_then(|t| be_u16(t, 44))
        .unwrap_or(0);
    let style_lower = style.to_ascii_lowercase();
    Some(FontFace {
        path: PathBuf::new(),
        family,
        style,
        weight,
        italic: selection & (1 | 512) != 0
            || mac_style & 2 != 0
            || style_lower.contains("italic")
            || style_lower.contains("oblique"),
        variable: table(data, face, b"fvar").is_some(),
    })
}

pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Remove the Internet-zone marker from an explicitly supplied font file.
pub fn unblock_file(path: &str) {
    let _ = fs::remove_file(format!("{path}:Zone.Identifier"));
}
