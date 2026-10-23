//! CS2 Font Changer — interactive Regular/Bold font deployment.

mod backup;
mod clipboard;
mod config;
mod console;
mod deploy;
mod dialog;
mod font;
mod game;
mod i18n;
mod logger;
mod registry;
mod settings;

#[cfg(test)]
mod tests;

use std::path::Path;

use crate::backup::{restore_backup, BACKUP_FILENAME};
use crate::console::{
    clear_screen, enter_label, input_discard, read_line, read_menu_key, wait_for_enter,
};
use crate::font::{FontFace, FontSelection};
use crate::game::{is_game_running, is_valid_install_location};
use crate::i18n::{t, t_with};

pub fn print_error(message: &str, exception: Option<&dyn std::error::Error>) {
    lprintln!("{}: {}", t("exception"), message);
    if let Some(e) = exception {
        lprintln!("{}: {}", t("diagnostic"), e);
    }
}

fn finish_execution(exit_code: i32) -> ! {
    lprint!("{}", t("task_complete"));
    loop {
        if let Some('\r' | '\n') = console::getch() {
            lprint!("\n");
            clipboard::copy_to_clipboard(&logger::get_logs_plain());
            lprintln!("{}", t("logs_copied"));
            std::process::exit(exit_code);
        }
    }
}

fn restore_selected(location: &mut Option<String>, backup: Option<&str>) {
    if !location
        .as_ref()
        .map(|p| is_valid_install_location(p))
        .unwrap_or(false)
    {
        lprintln!("{}", t("specify_path_first"));
        if let Some(path) = dialog::select_dir_dialog(&t("dialog_restore_title")) {
            *location = Some(path);
        } else {
            return;
        }
    }
    if let Some(path) = location.as_ref().filter(|p| is_valid_install_location(p)) {
        if restore_backup(path, backup) {
            finish_execution(0);
        }
        input_discard(&t("restore_failed_menu"));
    } else {
        input_discard(&t("invalid_path_specified"));
    }
}

fn main() {
    console::init_console();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut location = registry::get_auto_install_location();
    let mut fonts = FontSelection::default();
    let mut ui_scale = 1.0;

    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            match settings::load(&directory.join(settings::FILENAME), args.is_empty()) {
                Ok(Some(defaults)) => {
                    fonts = defaults.fonts;
                    ui_scale = defaults.ui_scale;
                }
                Ok(None) => {}
                Err(e) => {
                    print_error(&t("local_defaults_failed"), Some(&e));
                    wait_for_enter(&t("press_enter_menu"));
                }
            }
        }
    }

    if !args.is_empty() {
        if args.len() == 1
            && Path::new(&args[0])
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case(BACKUP_FILENAME))
                .unwrap_or(false)
        {
            lprintln!("{}", t("auto_restore_detect"));
            restore_selected(&mut location, Some(&args[0]));
        } else {
            match FontSelection::from_files(&args) {
                Some(selected) => {
                    for file in &args {
                        font::unblock_file(file);
                    }
                    fonts = selected;
                }
                None => {
                    print_error(&t("invalid_font_pair"), None);
                    wait_for_enter(&t("press_enter_menu"));
                }
            }
        }
    }

    loop {
        clear_screen();
        logger::clear();
        lprintln!("{}\n", t("menu_title"));
        if is_game_running() {
            lprintln!("{}", t("cs2_running"));
        }
        let has_backup = location
            .as_ref()
            .map(|p| Path::new(p).join(BACKUP_FILENAME).is_file())
            .unwrap_or(false);
        let can_start = fonts.validate()
            && location
                .as_ref()
                .map(|p| is_valid_install_location(p))
                .unwrap_or(false);
        let regular = fonts
            .regular
            .as_ref()
            .map(FontFace::label)
            .unwrap_or_else(|| t("not_selected"));
        let bold = fonts
            .bold
            .as_ref()
            .map(FontFace::label)
            .unwrap_or_else(|| t("bold_fallback"));
        lprintln!(
            "\x1b[96m[1]\x1b[0m {}: \x1b[90m{}\x1b[0m",
            t("menu_regular"),
            regular
        );
        lprintln!(
            "\x1b[96m[2]\x1b[0m {}: \x1b[90m{}\x1b[0m",
            t("menu_select_path"),
            location.as_deref().unwrap_or(&t("not_identified"))
        );
        lprintln!(
            "\x1b[96m[3]\x1b[0m {}: \x1b[90m{}\x1b[0m",
            t("menu_adjust_ui"),
            ui_scale
        );
        lprintln!(
            "\x1b[96m[4]\x1b[0m {}: \x1b[90m{}\x1b[0m",
            t("menu_bold"),
            bold
        );
        if fonts.bold.is_some() {
            lprintln!("\x1b[96m[5]\x1b[0m {}", t("menu_clear_bold"));
        }
        if has_backup {
            lprintln!("\x1b[96m[0]\x1b[0m {}", t("restore_default"));
        }
        if can_start {
            lprintln!("{}", t("menu_start"));
        }
        lprint!("\n> ");
        let mut keys = vec!['1', '2', '3', '4'];
        if fonts.bold.is_some() {
            keys.push('5');
        }
        if has_backup {
            keys.push('0');
        }
        if can_start {
            keys.push('\r');
        }
        match read_menu_key(&keys, &enter_label()).as_str() {
            key @ ("1" | "4") => {
                let bold = key == "4";
                let title = t(if bold {
                    "menu_bold"
                } else {
                    "select_font_or_restore"
                });
                let filters = if bold {
                    vec![(t("font_files"), "*.ttf;*.otf")]
                } else {
                    vec![(t("supported_files"), "*.ttf;*.otf;*.zip")]
                };
                let filters: Vec<_> = filters
                    .iter()
                    .map(|(label, pattern)| (label.as_str(), *pattern))
                    .collect();
                let Some(file) = dialog::select_file_dialog(&title, &filters) else {
                    continue;
                };
                if !bold
                    && Path::new(&file)
                        .extension()
                        .and_then(|s| s.to_str())
                        .map(|s| s.eq_ignore_ascii_case("zip"))
                        .unwrap_or(false)
                {
                    if Path::new(&file)
                        .file_name()
                        .and_then(|s| s.to_str())
                        .map(|s| s.eq_ignore_ascii_case(BACKUP_FILENAME))
                        .unwrap_or(false)
                    {
                        restore_selected(&mut location, Some(&file));
                    } else {
                        print_error(&t("wrong_zip_name"), None);
                        wait_for_enter(&t("press_enter_menu"));
                    }
                    continue;
                }
                let Some(face) = FontFace::load(Path::new(&file)) else {
                    print_error(&t_with("invalid_font_format", &[("file", &file)]), None);
                    wait_for_enter(&t("press_enter_menu"));
                    continue;
                };
                let candidate = FontSelection {
                    regular: if bold {
                        fonts.regular.clone()
                    } else {
                        Some(face.clone())
                    },
                    bold: if bold { Some(face) } else { fonts.bold.clone() },
                };
                if candidate.regular.is_none() || !candidate.validate() {
                    print_error(&t("invalid_font_pair"), None);
                    wait_for_enter(&t("press_enter_menu"));
                    continue;
                }
                fonts = candidate;
            }
            "2" => {
                if let Some(path) = dialog::select_dir_dialog(&t("dialog_game_title")) {
                    location = Some(path);
                }
            }
            "3" => loop {
                lprint!("{}", t("ui_scale_input"));
                let val = read_line();
                if val.is_empty() {
                    break;
                }
                match val.parse::<f64>() {
                    Ok(scale) if scale.is_finite() && (0.5..=2.0).contains(&scale) => {
                        ui_scale = scale;
                        break;
                    }
                    _ => lprintln!("{}", t("invalid_number")),
                }
            },
            "5" => fonts.bold = None,
            "0" => restore_selected(&mut location, None),
            "" if can_start => {
                let loc = location.as_ref().expect("validated game path");
                match deploy::apply(loc, &fonts, ui_scale) {
                    Ok(()) => {
                        lprintln!("{}", t("config_success"));
                        finish_execution(0);
                    }
                    Err(e) => {
                        print_error(&t("config_failed"), Some(&e));
                        finish_execution(1);
                    }
                }
            }
            _ => {}
        }
    }
}
