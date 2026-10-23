//! Deploy both faces together, retaining the previous state if a write fails.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::backup::create_backup;
use crate::config;
use crate::font::FontSelection;
use crate::game::get_fonts_paths;
use crate::i18n::{t, t_with};
use crate::lprintln;

pub fn apply(install_location: &str, fonts: &FontSelection, scale: f64) -> io::Result<()> {
    if !fonts.validate() || !scale.is_finite() || !(0.5..=2.0).contains(&scale) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            t("invalid_font_pair"),
        ));
    }
    let paths = get_fonts_paths(install_location);
    // Read both sources before touching the game: inputs may already be inside its font folder.
    let mut writes = Vec::new();
    for (face, bold) in [(fonts.regular.as_ref(), false), (fonts.bold.as_ref(), true)] {
        if let Some(face) = face {
            let bytes = fs::read(&face.path)?;
            if bytes.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    t("validation_failed_size"),
                ));
            }
            writes.push((paths.csgo_fonts.join(face.deployed_name(bold)), bytes));
        }
    }
    let configs = config::generate(fonts, scale);
    writes.push((
        paths.csgo_fonts.join("fonts.conf"),
        configs.local.into_bytes(),
    ));
    writes.push((
        paths.core_fonts.join("42-repl-global.conf"),
        configs.global.into_bytes(),
    ));

    create_backup(install_location)
        .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", t("backup_error"))))?;
    fs::create_dir_all(&paths.csgo_fonts)?;
    fs::create_dir_all(&paths.core_fonts)?;

    let mut previous = Vec::new();
    for entry in fs::read_dir(&paths.csgo_fonts)? {
        let path = entry?.path();
        let font = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.eq_ignore_ascii_case("ttf") || s.eq_ignore_ascii_case("otf"))
            .unwrap_or(false);
        if font || path == paths.ui_font {
            snapshot(&mut previous, &path)?;
        }
    }
    for (path, _) in &writes {
        snapshot(&mut previous, path)?;
    }

    let result = (|| -> io::Result<()> {
        for (path, bytes) in &previous {
            if bytes.is_some()
                && path.parent() == Some(paths.csgo_fonts.as_path())
                && path.file_name().and_then(|s| s.to_str()) != Some("fonts.conf")
            {
                fs::remove_file(path)?;
            }
        }
        for (path, bytes) in &writes {
            fs::write(path, bytes)?;
            lprintln!(
                "{}",
                t_with("file_deployed", &[("file", &path.to_string_lossy())])
            );
        }
        lprintln!("{}", t("validation_check"));
        for (path, expected) in &writes {
            if fs::read(path)? != *expected {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Verification failed: {}", path.display()),
                ));
            }
        }
        lprintln!("{}", t("validation_success"));
        Ok(())
    })();

    if let Err(error) = result {
        let mut rollback_error = None;
        for (path, bytes) in previous {
            let restored = match bytes {
                Some(bytes) => {
                    // A failed write may leave its target untouched (e.g. another process
                    // holds a read-only sharing lock). Do not rewrite an unchanged file.
                    if fs::read(&path).ok().as_deref() == Some(bytes.as_slice()) {
                        Ok(())
                    } else {
                        fs::write(&path, bytes)
                    }
                }
                None => match fs::remove_file(&path) {
                    Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                    result => result,
                },
            };
            if let Err(e) = restored {
                rollback_error = Some(e);
            }
        }
        return Err(match rollback_error {
            Some(e) => io::Error::new(
                error.kind(),
                format!("{error}; {}: {e}", t("rollback_failed")),
            ),
            None => {
                lprintln!("{}", t("rollback_complete"));
                error
            }
        });
    }
    Ok(())
}

fn snapshot(previous: &mut Vec<(PathBuf, Option<Vec<u8>>)>, path: &Path) -> io::Result<()> {
    if previous.iter().any(|(p, _)| p == path) {
        return Ok(());
    }
    let bytes = match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    previous.push((path.to_path_buf(), bytes));
    Ok(())
}
