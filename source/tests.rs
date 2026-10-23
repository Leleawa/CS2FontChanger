//! Regression tests use isolated fake game directories and synthetic font metadata.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::backup::{create_backup, restore_backup, BACKUP_FILENAME};
use crate::config;
use crate::deploy;
use crate::font::{FontFace, FontSelection};
use crate::game::get_fonts_paths;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-work");
        fs::create_dir_all(&base).unwrap();
        let dir = base.join(format!(
            "{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        Self(dir)
    }

    fn font(&self, filename: &str, weight: u16) -> FontFace {
        let path = self.0.join(filename);
        fs::write(
            &path,
            sfnt("Test Family", "Test Family", weight, false, false),
        )
        .unwrap();
        FontFace::load(&path).unwrap()
    }

    fn pair(&self) -> FontSelection {
        FontSelection {
            regular: Some(self.font("Regular.ttf", 400)),
            bold: Some(self.font("Bold.otf", 700)),
        }
    }

    fn game(&self) -> String {
        let root = self.0.join("Counter-Strike Global Offensive");
        let paths = get_fonts_paths(root.to_str().unwrap());
        fs::create_dir_all(&paths.csgo_fonts).unwrap();
        fs::create_dir_all(&paths.core_fonts).unwrap();
        fs::write(paths.csgo_fonts.join("original.ttf"), b"original font").unwrap();
        fs::write(paths.csgo_fonts.join("fonts.conf"), b"original config").unwrap();
        fs::write(paths.ui_font, b"original index").unwrap();
        root.to_str().unwrap().to_owned()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let intended = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-work");
        let resolved = self.0.canonicalize().unwrap();
        // Only remove a fixture directly within this project's dedicated test directory.
        assert_eq!(
            resolved.parent(),
            Some(intended.canonicalize().unwrap().as_path())
        );
        fs::remove_dir_all(resolved).unwrap();
    }
}

/// Build a minimal sfnt with real name and OS/2 table layouts. No glyph data is needed.
fn sfnt(legacy: &str, family: &str, weight: u16, italic: bool, variable: bool) -> Vec<u8> {
    let style = if italic {
        "Italic"
    } else if weight >= 600 {
        "Bold"
    } else {
        "Regular"
    };
    let strings = [(1, legacy), (2, style), (16, family), (17, style)];
    let mut names = vec![0; 6 + strings.len() * 12];
    names[2..4].copy_from_slice(&(strings.len() as u16).to_be_bytes());
    names[4..6].copy_from_slice(&((6 + strings.len() * 12) as u16).to_be_bytes());
    let mut payload = Vec::new();
    for (i, (id, value)) in strings.iter().enumerate() {
        let bytes: Vec<u8> = value.encode_utf16().flat_map(u16::to_be_bytes).collect();
        let record = [
            3u16,
            1,
            0x0409,
            *id,
            bytes.len() as u16,
            payload.len() as u16,
        ];
        for (j, item) in record.iter().enumerate() {
            names[6 + i * 12 + j * 2..8 + i * 12 + j * 2].copy_from_slice(&item.to_be_bytes());
        }
        payload.extend(bytes);
    }
    names.extend(payload);
    let mut os2 = vec![0; 64];
    os2[4..6].copy_from_slice(&weight.to_be_bytes());
    os2[62..64].copy_from_slice(&u16::from(italic).to_be_bytes());
    let mut tables = vec![(*b"name", names), (*b"OS/2", os2)];
    if variable {
        tables.push((*b"fvar", vec![0; 16]));
    }
    let mut font = vec![0; 12 + tables.len() * 16];
    font[..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    font[4..6].copy_from_slice(&(tables.len() as u16).to_be_bytes());
    for (i, (tag, bytes)) in tables.into_iter().enumerate() {
        let record = 12 + i * 16;
        let offset = font.len() as u32;
        font[record..record + 4].copy_from_slice(&tag);
        font[record + 8..record + 12].copy_from_slice(&offset.to_be_bytes());
        font[record + 12..record + 16].copy_from_slice(&(bytes.len() as u32).to_be_bytes());
        font.extend(bytes);
    }
    font
}

#[test]
fn reads_typographic_family_and_os2_weight() {
    let fixture = Fixture::new();
    let path = fixture.0.join("metadata.ttf");
    fs::write(
        &path,
        sfnt("Legacy Bold", "Unified Family", 700, false, false),
    )
    .unwrap();
    let face = FontFace::load(&path).unwrap();
    assert_eq!(face.family, "Unified Family");
    assert_eq!(face.style, "Bold");
    assert_eq!(face.weight, 700);
    assert!(!face.italic);
    assert!(!face.variable);
}

#[test]
fn local_defaults_resolve_relative_unicode_paths_and_both_weights() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.0.join("private fonts")).unwrap();
    fixture.font("private fonts/常规.ttf", 400);
    fixture.font("private fonts/粗体.otf", 700);
    let path = fixture.0.join(crate::settings::FILENAME);
    fs::write(&path, "\u{feff}# Local defaults\nregular_font = \"private fonts/常规.ttf\"\nbold_font = private fonts/粗体.otf\nui_scale = 0.9\n").unwrap();
    let defaults = crate::settings::load(&path, true).unwrap().unwrap();
    assert_eq!(defaults.ui_scale, 0.9);
    assert!(!defaults.test_mode);
    assert!(defaults.fonts.validate());
    assert_eq!(
        defaults.fonts.regular.unwrap().path,
        fixture.0.join("private fonts/常规.ttf")
    );
    assert_eq!(defaults.fonts.bold.unwrap().weight, 700);
}

#[test]
fn explicit_fonts_override_local_fonts_but_keep_local_scale() {
    let fixture = Fixture::new();
    let path = fixture.0.join(crate::settings::FILENAME);
    fs::write(
        &path,
        "regular_font=missing.ttf\nbold_font=missing-bold.ttf\nui_scale=0.9",
    )
    .unwrap();
    assert!(crate::settings::load(&path, true).is_err());
    let defaults = crate::settings::load(&path, false).unwrap().unwrap();
    assert_eq!(defaults.ui_scale, 0.9);
    assert!(!defaults.test_mode);
    assert!(defaults.fonts.regular.is_none());
    assert!(defaults.fonts.bold.is_none());
}

#[test]
fn missing_and_scale_only_local_defaults_preserve_standard_font_selection() {
    let fixture = Fixture::new();
    let path = fixture.0.join(crate::settings::FILENAME);
    assert!(crate::settings::load(&path, true).unwrap().is_none());
    fs::write(&path, "ui_scale=0.9").unwrap();
    let defaults = crate::settings::load(&path, true).unwrap().unwrap();
    assert_eq!(defaults.ui_scale, 0.9);
    assert!(!defaults.test_mode);
    assert!(defaults.fonts.regular.is_none());
}

#[test]
fn invalid_local_defaults_are_rejected() {
    let fixture = Fixture::new();
    let path = fixture.0.join(crate::settings::FILENAME);
    for text in [
        "ui_scale=NaN",
        "ui_scale=inf",
        "ui_scale=0.1",
        "ui_scale=2.1",
        "ui_scale=oops",
        "ui_scale=",
        "unknown=value",
        "test_mode=yes",
        "test_mode=1",
        "test_mode=true\ntest_mode=false",
        "not a setting",
        "ui_scale=1\nui_scale=0.9",
        "regular_font=\"unclosed",
    ] {
        fs::write(&path, text).unwrap();
        assert!(crate::settings::load(&path, true).is_err(), "{text}");
    }
    let bold = fixture.font("only-bold.ttf", 700);
    fs::write(&path, format!("bold_font={}", bold.path.display())).unwrap();
    assert!(crate::settings::load(&path, true).is_err());
}

#[test]
fn invalid_or_truncated_fonts_are_rejected() {
    let fixture = Fixture::new();
    let bytes = sfnt("Legacy", "Family", 400, false, false);
    let path = fixture.0.join("invalid.ttf");
    // Exercise every truncation, including a partial table directory/name record.
    for length in 0..bytes.len() {
        fs::write(&path, &bytes[..length]).unwrap();
        // A truncated optional OS/2 table may legitimately fall back to style metadata.
        if length < 44 {
            assert!(FontFace::load(&path).is_none());
        }
    }
    fs::write(&path, b"not a font").unwrap();
    assert!(FontFace::load(&path).is_none());
    fs::write(fixture.0.join("font.zip"), &bytes).unwrap();
    assert!(FontFace::load(&fixture.0.join("font.zip")).is_none());
}

#[test]
fn drag_pair_is_order_independent_and_invalid_args_do_not_enable_start() {
    let fixture = Fixture::new();
    let pair = fixture.pair();
    let args = vec![
        pair.bold.unwrap().path.to_str().unwrap().to_string(),
        pair.regular.unwrap().path.to_str().unwrap().to_string(),
    ];
    let selected = FontSelection::from_files(&args).unwrap();
    assert_eq!(selected.regular.unwrap().weight, 400);
    assert_eq!(selected.bold.unwrap().weight, 700);
    assert!(FontSelection::from_files(&[]).is_none());
    assert!(FontSelection::from_files(&["missing.ttf".into()]).is_none());
    assert!(FontSelection::from_files(&[args[0].clone(), args[0].clone()]).is_none());
    assert!(
        FontSelection::from_files(&[args[0].clone(), args[0].clone(), args[1].clone()]).is_none()
    );
}

#[test]
fn single_font_accepts_legacy_bold_but_pairs_reject_wrong_weights_italic_and_variable() {
    let fixture = Fixture::new();
    let mut fonts = FontSelection {
        regular: Some(fixture.font("single.ttf", 700)),
        bold: None,
    };
    assert!(fonts.validate());
    fonts.bold = Some(fixture.font("bold.ttf", 700));
    assert!(!fonts.validate());
    fonts.regular = Some(fixture.font("regular.ttf", 400));
    assert!(fonts.validate());
    for (italic, variable) in [(true, false), (false, true)] {
        let path = fixture.0.join("variant.ttf");
        fs::write(&path, sfnt("Test", "Test", 700, italic, variable)).unwrap();
        fonts.bold = FontFace::load(&path);
        assert!(!fonts.validate());
    }
}

#[test]
fn paired_deployment_preserves_both_faces_and_restores_originals() {
    let fixture = Fixture::new();
    let fonts = fixture.pair();
    let game = fixture.game();
    let paths = get_fonts_paths(&game);
    deploy::apply(&game, &fonts, 1.1, false).unwrap();
    assert_eq!(
        fs::read(paths.csgo_fonts.join("cs2fc-regular.ttf")).unwrap(),
        fs::read(&fonts.regular.as_ref().unwrap().path).unwrap()
    );
    assert_eq!(
        fs::read(paths.csgo_fonts.join("cs2fc-bold.otf")).unwrap(),
        fs::read(&fonts.bold.as_ref().unwrap().path).unwrap()
    );
    assert!(!paths.csgo_fonts.join("original.ttf").exists());
    let config = fs::read_to_string(paths.core_fonts.join("42-repl-global.conf")).unwrap();
    assert!(config.contains("<const>demibold</const>"));
    assert!(restore_backup(&game, None));
    assert_eq!(
        fs::read(paths.csgo_fonts.join("original.ttf")).unwrap(),
        b"original font"
    );
    assert_eq!(
        fs::read(paths.csgo_fonts.join("fonts.conf")).unwrap(),
        b"original config"
    );
    assert_eq!(fs::read(paths.ui_font).unwrap(), b"original index");
    assert!(!paths.csgo_fonts.join("cs2fc-bold.otf").exists());
    assert!(!paths.core_fonts.join("42-repl-global.conf").exists());
}

#[test]
fn switching_to_single_font_removes_bold_and_keeps_first_backup() {
    let fixture = Fixture::new();
    let mut fonts = fixture.pair();
    let game = fixture.game();
    let paths = get_fonts_paths(&game);
    deploy::apply(&game, &fonts, 1.0, false).unwrap();
    let backup = fs::read(Path::new(&game).join(BACKUP_FILENAME)).unwrap();
    fonts.bold = None;
    deploy::apply(&game, &fonts, 1.0, false).unwrap();
    assert!(!paths.csgo_fonts.join("cs2fc-bold.otf").exists());
    assert_eq!(
        fs::read(Path::new(&game).join(BACKUP_FILENAME)).unwrap(),
        backup
    );
}

#[test]
fn sources_inside_game_directory_are_read_before_cleanup() {
    let fixture = Fixture::new();
    let game = fixture.game();
    let paths = get_fonts_paths(&game);
    let regular_path = paths.csgo_fonts.join("input.TTF");
    fs::write(&regular_path, sfnt("Test", "Test", 400, false, false)).unwrap();
    let fonts = FontSelection {
        regular: FontFace::load(&regular_path),
        bold: None,
    };
    deploy::apply(&game, &fonts, 1.0, false).unwrap();
    assert!(paths.csgo_fonts.join("cs2fc-regular.ttf").is_file());
    assert!(!regular_path.exists());
}

#[test]
fn missing_source_or_corrupt_backup_never_cleans_game_fonts() {
    let fixture = Fixture::new();
    let fonts = fixture.pair();
    let game = fixture.game();
    let paths = get_fonts_paths(&game);
    fs::remove_file(&fonts.bold.as_ref().unwrap().path).unwrap();
    assert!(deploy::apply(&game, &fonts, 1.0, false).is_err());
    assert_eq!(
        fs::read(paths.csgo_fonts.join("original.ttf")).unwrap(),
        b"original font"
    );
    let fonts = fixture.pair();
    fs::write(Path::new(&game).join(BACKUP_FILENAME), b"broken zip").unwrap();
    assert!(deploy::apply(&game, &fonts, 1.0, false).is_err());
    assert_eq!(
        fs::read(paths.csgo_fonts.join("original.ttf")).unwrap(),
        b"original font"
    );
}

#[test]
fn failed_config_write_keeps_previous_fonts() {
    let fixture = Fixture::new();
    let fonts = fixture.pair();
    let game = fixture.game();
    let paths = get_fonts_paths(&game);
    fs::create_dir(paths.core_fonts.join("42-repl-global.conf")).unwrap();
    assert!(deploy::apply(&game, &fonts, 1.0, false).is_err());
    assert_eq!(
        fs::read(paths.csgo_fonts.join("original.ttf")).unwrap(),
        b"original font"
    );
    assert_eq!(
        fs::read(paths.csgo_fonts.join("fonts.conf")).unwrap(),
        b"original config"
    );
}

#[test]
fn incomplete_backup_is_not_published() {
    let fixture = Fixture::new();
    let game = fixture.game();
    // A directory at the partial-file path simulates failure to create/write the archive.
    let partial = Path::new(&game).join("backup_original_fonts.zip.partial");
    fs::create_dir(&partial).unwrap();
    assert!(create_backup(&game).is_err());
    assert!(!Path::new(&game).join(BACKUP_FILENAME).exists());
    assert!(get_fonts_paths(&game)
        .csgo_fonts
        .join("original.ttf")
        .is_file());
}

#[test]
#[cfg(windows)]
fn locked_global_config_rolls_back_after_fonts_and_local_config_were_written() {
    use std::os::windows::fs::OpenOptionsExt;

    let fixture = Fixture::new();
    let fonts = fixture.pair();
    let game = fixture.game();
    let paths = get_fonts_paths(&game);
    let global = paths.core_fonts.join("42-repl-global.conf");
    fs::write(&global, b"previous global config").unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&global)
        .unwrap();
    assert!(deploy::apply(&game, &fonts, 1.0, false).is_err());
    assert_eq!(
        fs::read(paths.csgo_fonts.join("original.ttf")).unwrap(),
        b"original font"
    );
    assert_eq!(
        fs::read(paths.csgo_fonts.join("fonts.conf")).unwrap(),
        b"original config"
    );
    assert_eq!(fs::read(paths.ui_font).unwrap(), b"original index");
    assert_eq!(fs::read(&global).unwrap(), b"previous global config");
    assert!(!paths.csgo_fonts.join("cs2fc-regular.ttf").exists());
    assert!(!paths.csgo_fonts.join("cs2fc-bold.otf").exists());
    drop(lock);
}

fn write_zip(path: &Path, name: &str) {
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    zip.start_file(
        name,
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored),
    )
    .unwrap();
    zip.write_all(b"unique-original-payload").unwrap();
    zip.finish().unwrap();
}

#[test]
fn corrupt_payload_and_unsafe_paths_are_rejected_before_restore_cleanup() {
    let fixture = Fixture::new();
    let game = fixture.game();
    let archive = Path::new(&game).join(BACKUP_FILENAME);
    write_zip(&archive, "csgo_fonts/font.ttf");
    let mut bytes = fs::read(&archive).unwrap();
    let offset = bytes
        .windows(b"unique-original-payload".len())
        .position(|b| b == b"unique-original-payload")
        .unwrap();
    bytes[offset] ^= 0xff;
    fs::write(&archive, bytes).unwrap();
    assert!(!restore_backup(&game, None));
    for name in [
        "csgo_fonts/../outside.ttf",
        "csgo_fonts/C:outside.ttf",
        "csgo_fonts//absolute.ttf",
        "unrelated/file.ttf",
    ] {
        write_zip(&archive, name);
        assert!(!restore_backup(&game, None));
        assert_eq!(
            fs::read(get_fonts_paths(&game).csgo_fonts.join("original.ttf")).unwrap(),
            b"original font"
        );
    }
}

#[test]
fn config_escapes_families_and_has_one_scaling_rule_per_face() {
    let fixture = Fixture::new();
    let mut fonts = fixture.pair();
    fonts.regular.as_mut().unwrap().family = "Regular & <Family>".into();
    fonts.bold.as_mut().unwrap().family = "Different Bold Family".into();
    let configs = config::generate(&fonts, 1.1);
    assert!(configs.local.contains("Regular &amp; &lt;Family&gt;"));
    assert!(configs.local.contains("<fontpattern>.otf</fontpattern>"));
    assert!(configs
        .global
        .contains("<string>Different Bold Family</string>"));
    assert_eq!(configs.global.matches("<double>1.1</double>").count(), 2);
    assert!(!configs.local.contains("<double>1.1</double>"));
    assert!(!configs.local.contains("mode=\"assign_replace\""));
    assert_eq!(configs.global.matches("<edit name=\"family\"").count(), 1);
    // Produce reviewable XML for the independent XML-parser check.
    let output = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/fontconfig-preview");
    fs::create_dir_all(&output).unwrap();
    fs::write(output.join("fonts.conf"), configs.local).unwrap();
    fs::write(output.join("42-repl-global.conf"), configs.global).unwrap();
}

#[test]
fn reads_real_windows_regular_and_bold_fonts() {
    let system = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:/Windows"));
    let regular = FontFace::load(&system.join("Fonts/arial.ttf")).unwrap();
    let bold = FontFace::load(&system.join("Fonts/arialbd.ttf")).unwrap();
    assert_eq!(regular.family, bold.family);
    assert_eq!(regular.weight, 400);
    assert_eq!(bold.weight, 700);
    let mut fonts = FontSelection {
        regular: Some(regular),
        bold: Some(bold),
    };
    assert!(fonts.validate());
    // Native-engine integration checks consume these fixtures without touching the game.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/fontconfig-native");
    for scenario in ["same-family", "different-family", "single"] {
        if scenario == "different-family" {
            fonts.bold = FontFace::load(&system.join("Fonts/courbd.ttf"));
            assert!(fonts.validate());
        } else if scenario == "single" {
            fonts.bold = None;
        }
        let directory = root.join(scenario);
        fs::create_dir_all(&directory).unwrap();
        let configs = config::generate(&fonts, 1.1);
        fs::write(directory.join("fonts.conf"), configs.local).unwrap();
        fs::write(directory.join("42-repl-global.conf"), configs.global).unwrap();
        for (face, bold) in [(fonts.regular.as_ref(), false), (fonts.bold.as_ref(), true)] {
            if let Some(face) = face {
                fs::copy(&face.path, directory.join(face.deployed_name(bold))).unwrap();
            }
        }
    }
}

#[test]
fn test_mode_defaults_are_validated_even_when_fonts_are_overridden() {
    let fixture = Fixture::new();
    let path = fixture.0.join(crate::settings::FILENAME);
    for (value, expected) in [("true", true), ("false", false)] {
        fs::write(
            &path,
            format!("regular_font=missing.ttf\nui_scale=0.9\ntest_mode={value}"),
        )
        .unwrap();
        let defaults = crate::settings::load(&path, false).unwrap().unwrap();
        assert_eq!(defaults.test_mode, expected);
        assert_eq!(defaults.ui_scale, 0.9);
    }
    fs::write(&path, "test_mode=true").unwrap();
    assert!(
        crate::settings::load(&path, true)
            .unwrap()
            .unwrap()
            .test_mode
    );
    fs::write(&path, "test_mode=invalid").unwrap();
    assert!(crate::settings::load(&path, false).is_err());
}

#[test]
fn test_mode_preserves_originals_across_font_and_weight_switches_and_restore() {
    let fixture = Fixture::new();
    let mut fonts = fixture.pair();
    let game = fixture.game();
    let paths = get_fonts_paths(&game);
    let foreign = paths.csgo_fonts.join("other-tool.OTF");
    fs::write(&foreign, b"unrelated font").unwrap();
    deploy::apply(&game, &fonts, 0.9, true).unwrap();
    let backup = fs::read(Path::new(&game).join(BACKUP_FILENAME)).unwrap();
    assert_eq!(
        fs::read(paths.csgo_fonts.join("cs2fc-regular.ttf")).unwrap(),
        fs::read(&fonts.regular.as_ref().unwrap().path).unwrap()
    );
    assert_eq!(
        fs::read(paths.csgo_fonts.join("cs2fc-bold.otf")).unwrap(),
        fs::read(&fonts.bold.as_ref().unwrap().path).unwrap()
    );
    // Change format and drop bold: only the program's stale outputs may be removed.
    fonts.regular = Some(fixture.font("replacement.otf", 400));
    fonts.bold = None;
    deploy::apply(&game, &fonts, 1.0, true).unwrap();
    assert!(!paths.csgo_fonts.join("cs2fc-regular.ttf").exists());
    assert!(!paths.csgo_fonts.join("cs2fc-bold.otf").exists());
    assert!(paths.csgo_fonts.join("cs2fc-regular.otf").exists());
    assert_eq!(
        fs::read(paths.csgo_fonts.join("original.ttf")).unwrap(),
        b"original font"
    );
    assert_eq!(fs::read(&paths.ui_font).unwrap(), b"original index");
    assert_eq!(fs::read(&foreign).unwrap(), b"unrelated font");
    assert_eq!(
        fs::read(Path::new(&game).join(BACKUP_FILENAME)).unwrap(),
        backup
    );
    let generated = config::generate(&fonts, 1.0);
    assert_eq!(
        fs::read_to_string(paths.csgo_fonts.join("fonts.conf")).unwrap(),
        generated.local
    );
    assert_eq!(
        fs::read_to_string(paths.core_fonts.join("42-repl-global.conf")).unwrap(),
        generated.global
    );
    assert!(restore_backup(&game, None));
    assert_eq!(
        fs::read(paths.csgo_fonts.join("original.ttf")).unwrap(),
        b"original font"
    );
    assert_eq!(fs::read(&paths.ui_font).unwrap(), b"original index");
    assert_eq!(fs::read(&foreign).unwrap(), b"unrelated font");
    assert_eq!(
        fs::read(paths.csgo_fonts.join("fonts.conf")).unwrap(),
        b"original config"
    );
    assert!(!paths.csgo_fonts.join("cs2fc-regular.otf").exists());
}

#[test]
#[cfg(windows)]
fn test_mode_failed_redeployment_restores_previous_custom_fonts_and_keeps_originals() {
    use std::os::windows::fs::OpenOptionsExt;
    let fixture = Fixture::new();
    let mut fonts = fixture.pair();
    let game = fixture.game();
    let paths = get_fonts_paths(&game);
    deploy::apply(&game, &fonts, 0.9, true).unwrap();
    let regular = paths.csgo_fonts.join("cs2fc-regular.ttf");
    let bold = paths.csgo_fonts.join("cs2fc-bold.otf");
    let local = paths.csgo_fonts.join("fonts.conf");
    let global = paths.core_fonts.join("42-repl-global.conf");
    let originals: Vec<_> = [&regular, &bold, &local, &global]
        .into_iter()
        .map(|p| (p, fs::read(p).unwrap()))
        .collect();
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&global)
        .unwrap();
    fonts.regular = Some(fixture.font("replacement.otf", 400));
    fonts.bold = None;
    assert!(deploy::apply(&game, &fonts, 1.2, true).is_err());
    for (path, bytes) in originals {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    assert!(!paths.csgo_fonts.join("cs2fc-regular.otf").exists());
    assert_eq!(
        fs::read(paths.csgo_fonts.join("original.ttf")).unwrap(),
        b"original font"
    );
    assert_eq!(fs::read(paths.ui_font).unwrap(), b"original index");
    drop(lock);
}
