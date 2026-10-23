//! Optional machine-local defaults, stored beside the executable.

use std::fs;
use std::io;
use std::path::Path;

use crate::font::{FontFace, FontSelection};

pub const FILENAME: &str = "cs2_font_changer.local.conf";

pub struct Defaults {
    pub fonts: FontSelection,
    pub ui_scale: f64,
}

/// Relative font paths resolve from the config file, independent of the working
/// directory. Explicit command-line fonts override the configured font selection.
pub fn load(path: &Path, use_font_defaults: bool) -> io::Result<Option<Defaults>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut regular = None;
    let mut bold = None;
    let mut scale = None;
    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or_else(invalid_config)?;
        let value = value.trim();
        let value = if value.starts_with('"') || value.ends_with('"') {
            value
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .ok_or_else(invalid_config)?
        } else {
            value
        };
        if value.is_empty() {
            return Err(invalid_config());
        }
        let entry = match key.trim() {
            "regular_font" => &mut regular,
            "bold_font" => &mut bold,
            "ui_scale" => &mut scale,
            _ => return Err(invalid_config()),
        };
        if entry.replace(value.to_string()).is_some() {
            return Err(invalid_config());
        }
    }
    let ui_scale = match scale {
        Some(value) => value.parse::<f64>().map_err(|_| invalid_config())?,
        None => 1.0,
    };
    if !ui_scale.is_finite() || !(0.5..=2.0).contains(&ui_scale) {
        return Err(invalid_config());
    }

    let mut fonts = FontSelection::default();
    if use_font_defaults {
        let base = path.parent().unwrap_or(Path::new("."));
        let load_font = |file: String| {
            let font_path = base.join(file);
            FontFace::load(&font_path).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Invalid local font: {}", font_path.display()),
                )
            })
        };
        fonts.regular = regular.map(&load_font).transpose()?;
        fonts.bold = bold.map(load_font).transpose()?;
        if (fonts.regular.is_some() || fonts.bold.is_some()) && !fonts.validate() {
            return Err(invalid_config());
        }
    }
    Ok(Some(Defaults { fonts, ui_scale }))
}

fn invalid_config() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid local defaults: use regular_font, optional bold_font, and ui_scale (0.5..=2.0)",
    )
}
