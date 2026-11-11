//! Turning a captured key press into a hotkey the OS can register.
//!
//! The two platforms do not share key codes: Windows wants a virtual-key code, macOS a
//! Carbon virtual key code, and both are stored in `settings.json` (as the Python app
//! stored them). The macOS values below are `kVK_*` from the Carbon headers; the Windows
//! ones are the `VK_*` constants, named rather than written out, so a typo is a compile
//! error instead of a hotkey that silently never fires.

use eframe::egui;

use rewind_engine::Hotkey;

/// Win32 `MOD_*` flags, the same values `settings.json` has always used.
pub const MOD_ALT: u32 = 1;
pub const MOD_CONTROL: u32 = 2;
pub const MOD_SHIFT: u32 = 4;
// Windows cannot report the Win key through egui, so nothing produces this flag there —
// but existing settings may still carry it, and registration honours it.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const MOD_WIN: u32 = 8;

/// Build the hotkey a press describes, or `None` for a key that cannot be one.
pub fn capture(key: egui::Key, modifiers: &egui::Modifiers) -> Option<Hotkey> {
    let (mods, symbols) = modifier_flags(modifiers);
    let (vk, name) = code(key)?;
    Some(Hotkey {
        mods,
        vk,
        text: format_combo(&symbols, name),
    })
}

/// The modifier flags, and how they are written.
fn modifier_flags(modifiers: &egui::Modifiers) -> (u32, Vec<&'static str>) {
    #[cfg(target_os = "macos")]
    {
        let mut mods = 0;
        let mut symbols = Vec::new();
        if modifiers.ctrl {
            mods |= MOD_CONTROL;
            symbols.push("\u{2303}"); // ⌃
        }
        if modifiers.alt {
            mods |= MOD_ALT;
            symbols.push("\u{2325}"); // ⌥
        }
        if modifiers.shift {
            mods |= MOD_SHIFT;
            symbols.push("\u{21e7}"); // ⇧
        }
        if modifiers.command {
            mods |= MOD_WIN;
            symbols.push("\u{2318}"); // ⌘
        }
        (mods, symbols)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut mods = 0;
        let mut symbols = Vec::new();
        if modifiers.ctrl {
            mods |= MOD_CONTROL;
            symbols.push("Ctrl");
        }
        if modifiers.alt {
            mods |= MOD_ALT;
            symbols.push("Alt");
        }
        if modifiers.shift {
            mods |= MOD_SHIFT;
            symbols.push("Shift");
        }
        (mods, symbols)
    }
}

/// `⌃⌥S` on macOS, `Ctrl + Alt + S` elsewhere.
fn format_combo(symbols: &[&str], name: &str) -> String {
    if cfg!(target_os = "macos") {
        let mut text: String = symbols.concat();
        text.push_str(name);
        text
    } else {
        let mut text = symbols.join(" + ");
        if !text.is_empty() {
            text.push_str(" + ");
        }
        text.push_str(name);
        text
    }
}

#[cfg(target_os = "macos")]
fn code(key: egui::Key) -> Option<(u32, &'static str)> {
    use egui::Key;
    match key {
        Key::A => Some((0, "A")),
        Key::B => Some((11, "B")),
        Key::C => Some((8, "C")),
        Key::D => Some((2, "D")),
        Key::E => Some((14, "E")),
        Key::F => Some((3, "F")),
        Key::G => Some((5, "G")),
        Key::H => Some((4, "H")),
        Key::I => Some((34, "I")),
        Key::J => Some((38, "J")),
        Key::K => Some((40, "K")),
        Key::L => Some((37, "L")),
        Key::M => Some((46, "M")),
        Key::N => Some((45, "N")),
        Key::O => Some((31, "O")),
        Key::P => Some((35, "P")),
        Key::Q => Some((12, "Q")),
        Key::R => Some((15, "R")),
        Key::S => Some((1, "S")),
        Key::T => Some((17, "T")),
        Key::U => Some((32, "U")),
        Key::V => Some((9, "V")),
        Key::W => Some((13, "W")),
        Key::X => Some((7, "X")),
        Key::Y => Some((16, "Y")),
        Key::Z => Some((6, "Z")),
        Key::Num0 => Some((29, "0")),
        Key::Num1 => Some((18, "1")),
        Key::Num2 => Some((19, "2")),
        Key::Num3 => Some((20, "3")),
        Key::Num4 => Some((21, "4")),
        Key::Num5 => Some((23, "5")),
        Key::Num6 => Some((22, "6")),
        Key::Num7 => Some((26, "7")),
        Key::Num8 => Some((28, "8")),
        Key::Num9 => Some((25, "9")),
        Key::F1 => Some((122, "F1")),
        Key::F2 => Some((120, "F2")),
        Key::F3 => Some((99, "F3")),
        Key::F4 => Some((118, "F4")),
        Key::F5 => Some((96, "F5")),
        Key::F6 => Some((97, "F6")),
        Key::F7 => Some((98, "F7")),
        Key::F8 => Some((100, "F8")),
        Key::F9 => Some((101, "F9")),
        Key::F10 => Some((109, "F10")),
        Key::F11 => Some((103, "F11")),
        Key::F12 => Some((111, "F12")),
        Key::F13 => Some((105, "F13")),
        Key::Space => Some((49, "Space")),
        Key::Tab => Some((48, "Tab")),
        Key::Enter => Some((36, "Enter")),
        Key::Backspace => Some((51, "Backspace")),
        Key::Delete => Some((117, "Delete")),
        Key::Home => Some((115, "Home")),
        Key::End => Some((119, "End")),
        Key::PageUp => Some((116, "PgUp")),
        Key::PageDown => Some((121, "PgDn")),
        Key::ArrowLeft => Some((123, "Left")),
        Key::ArrowRight => Some((124, "Right")),
        Key::ArrowUp => Some((126, "Up")),
        Key::ArrowDown => Some((125, "Down")),
        Key::Minus => Some((27, "-")),
        Key::Equals => Some((24, "=")),
        Key::Plus => Some((24, "=")),
        Key::OpenBracket => Some((33, "[")),
        Key::CloseBracket => Some((30, "]")),
        Key::Backslash => Some((42, "\\\\")),
        Key::Semicolon => Some((41, ";")),
        Key::Quote => Some((39, "'")),
        Key::Backtick => Some((50, "`")),
        Key::Comma => Some((43, ",")),
        Key::Period => Some((47, ".")),
        Key::Slash => Some((44, "/")),
        Key::Colon => Some((41, ";")),
        Key::Pipe => Some((42, "\\\\")),
        Key::Questionmark => Some((44, "/")),
        Key::Exclamationmark => Some((18, "1")),
        _ => None,
    }
}

#[cfg(target_os = "windows")]
fn code(key: egui::Key) -> Option<(u32, &'static str)> {
    use egui::Key;
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    match key {
        Key::A => Some((VK_A.0 as u32, "A")),
        Key::B => Some((VK_B.0 as u32, "B")),
        Key::C => Some((VK_C.0 as u32, "C")),
        Key::D => Some((VK_D.0 as u32, "D")),
        Key::E => Some((VK_E.0 as u32, "E")),
        Key::F => Some((VK_F.0 as u32, "F")),
        Key::G => Some((VK_G.0 as u32, "G")),
        Key::H => Some((VK_H.0 as u32, "H")),
        Key::I => Some((VK_I.0 as u32, "I")),
        Key::J => Some((VK_J.0 as u32, "J")),
        Key::K => Some((VK_K.0 as u32, "K")),
        Key::L => Some((VK_L.0 as u32, "L")),
        Key::M => Some((VK_M.0 as u32, "M")),
        Key::N => Some((VK_N.0 as u32, "N")),
        Key::O => Some((VK_O.0 as u32, "O")),
        Key::P => Some((VK_P.0 as u32, "P")),
        Key::Q => Some((VK_Q.0 as u32, "Q")),
        Key::R => Some((VK_R.0 as u32, "R")),
        Key::S => Some((VK_S.0 as u32, "S")),
        Key::T => Some((VK_T.0 as u32, "T")),
        Key::U => Some((VK_U.0 as u32, "U")),
        Key::V => Some((VK_V.0 as u32, "V")),
        Key::W => Some((VK_W.0 as u32, "W")),
        Key::X => Some((VK_X.0 as u32, "X")),
        Key::Y => Some((VK_Y.0 as u32, "Y")),
        Key::Z => Some((VK_Z.0 as u32, "Z")),
        Key::Num0 => Some((VK_0.0 as u32, "0")),
        Key::Num1 => Some((VK_1.0 as u32, "1")),
        Key::Num2 => Some((VK_2.0 as u32, "2")),
        Key::Num3 => Some((VK_3.0 as u32, "3")),
        Key::Num4 => Some((VK_4.0 as u32, "4")),
        Key::Num5 => Some((VK_5.0 as u32, "5")),
        Key::Num6 => Some((VK_6.0 as u32, "6")),
        Key::Num7 => Some((VK_7.0 as u32, "7")),
        Key::Num8 => Some((VK_8.0 as u32, "8")),
        Key::Num9 => Some((VK_9.0 as u32, "9")),
        Key::F1 => Some((VK_F1.0 as u32, "F1")),
        Key::F2 => Some((VK_F2.0 as u32, "F2")),
        Key::F3 => Some((VK_F3.0 as u32, "F3")),
        Key::F4 => Some((VK_F4.0 as u32, "F4")),
        Key::F5 => Some((VK_F5.0 as u32, "F5")),
        Key::F6 => Some((VK_F6.0 as u32, "F6")),
        Key::F7 => Some((VK_F7.0 as u32, "F7")),
        Key::F8 => Some((VK_F8.0 as u32, "F8")),
        Key::F9 => Some((VK_F9.0 as u32, "F9")),
        Key::F10 => Some((VK_F10.0 as u32, "F10")),
        Key::F11 => Some((VK_F11.0 as u32, "F11")),
        Key::F12 => Some((VK_F12.0 as u32, "F12")),
        Key::F13 => Some((VK_F13.0 as u32, "F13")),
        Key::Space => Some((VK_SPACE.0 as u32, "Space")),
        Key::Tab => Some((VK_TAB.0 as u32, "Tab")),
        Key::Enter => Some((VK_RETURN.0 as u32, "Enter")),
        Key::Backspace => Some((VK_BACK.0 as u32, "Backspace")),
        Key::Delete => Some((VK_DELETE.0 as u32, "Delete")),
        Key::Home => Some((VK_HOME.0 as u32, "Home")),
        Key::End => Some((VK_END.0 as u32, "End")),
        Key::PageUp => Some((VK_PRIOR.0 as u32, "PgUp")),
        Key::PageDown => Some((VK_NEXT.0 as u32, "PgDn")),
        Key::ArrowLeft => Some((VK_LEFT.0 as u32, "Left")),
        Key::ArrowRight => Some((VK_RIGHT.0 as u32, "Right")),
        Key::ArrowUp => Some((VK_UP.0 as u32, "Up")),
        Key::ArrowDown => Some((VK_DOWN.0 as u32, "Down")),
        Key::Minus => Some((VK_OEM_MINUS.0 as u32, "-")),
        Key::Equals => Some((VK_OEM_PLUS.0 as u32, "=")),
        Key::Plus => Some((VK_OEM_PLUS.0 as u32, "=")),
        Key::OpenBracket => Some((VK_OEM_4.0 as u32, "[")),
        Key::CloseBracket => Some((VK_OEM_6.0 as u32, "]")),
        Key::Backslash => Some((VK_OEM_5.0 as u32, "\\\\")),
        Key::Semicolon => Some((VK_OEM_1.0 as u32, ";")),
        Key::Quote => Some((VK_OEM_7.0 as u32, "'")),
        Key::Backtick => Some((VK_OEM_3.0 as u32, "`")),
        Key::Comma => Some((VK_OEM_COMMA.0 as u32, ",")),
        Key::Period => Some((VK_OEM_PERIOD.0 as u32, ".")),
        Key::Slash => Some((VK_OEM_2.0 as u32, "/")),
        Key::Colon => Some((VK_OEM_1.0 as u32, ";")),
        Key::Pipe => Some((VK_OEM_5.0 as u32, "\\\\")),
        Key::Questionmark => Some((VK_OEM_2.0 as u32, "/")),
        Key::Exclamationmark => Some((VK_1.0 as u32, "1")),
        _ => None,
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn code(_key: egui::Key) -> Option<(u32, &'static str)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(ctrl: bool, alt: bool, shift: bool, command: bool) -> egui::Modifiers {
        egui::Modifiers {
            alt,
            ctrl,
            shift,
            mac_cmd: command,
            command,
        }
    }

    #[test]
    fn a_letter_carries_its_modifiers_and_name() {
        let hotkey = capture(egui::Key::S, &mods(true, true, false, false)).unwrap();
        #[cfg(target_os = "macos")]
        {
            assert_eq!(hotkey.text, "\u{2303}\u{2325}S");
            assert_eq!(hotkey.vk, 1); // kVK_ANSI_S
        }
        #[cfg(not(target_os = "macos"))]
        {
            assert_eq!(hotkey.text, "Ctrl + Alt + S");
            assert_eq!(hotkey.vk, 0x53);
        }
        assert_eq!(hotkey.mods, MOD_CONTROL | MOD_ALT);
    }

    #[test]
    fn every_supported_key_has_a_code_and_a_name() {
        let keys = [
            egui::Key::A,
            egui::Key::Z,
            egui::Key::Num0,
            egui::Key::F1,
            egui::Key::F12,
            egui::Key::Space,
            egui::Key::Enter,
            egui::Key::ArrowLeft,
            egui::Key::Slash,
            egui::Key::Backtick,
        ];
        for key in keys {
            let (_, name) = code(key).unwrap_or_else(|| panic!("{key:?} has no code"));
            assert!(!name.is_empty(), "{key:?} has no name");
        }
        // Escape cancels capture; modifier keys are not hotkeys on their own.
        assert!(code(egui::Key::Escape).is_none());
        assert!(code(egui::Key::ShiftLeft).is_none());
    }

    #[test]
    fn the_shipped_defaults_come_from_this_table() {
        // The defaults in `settings.rs` were transcribed from the Python app; the table
        // here was taken from the platform headers. They must agree, or the Hotkeys page
        // would show one combo and register another.
        let (toggle, clip, record, shot, open) = rewind_engine::default_hotkeys();
        #[cfg(target_os = "macos")]
        let keys = [
            (egui::Key::I, toggle),
            (egui::Key::S, clip),
            (egui::Key::R, record),
            (egui::Key::P, shot),
            (egui::Key::Num0, open),
        ];
        #[cfg(not(target_os = "macos"))]
        let keys = [
            (egui::Key::F9, toggle),
            (egui::Key::F10, clip),
            (egui::Key::F11, record),
            (egui::Key::F12, shot),
            (egui::Key::Num0, open),
        ];
        for (key, hotkey) in keys {
            assert_eq!(code(key).unwrap().0, hotkey.vk, "{key:?}");
        }
    }

    #[test]
    fn each_key_gets_its_own_code() {
        let mut seen = std::collections::HashMap::new();
        for key in [
            egui::Key::A,
            egui::Key::B,
            egui::Key::C,
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::F1,
            egui::Key::F2,
            egui::Key::Space,
            egui::Key::Tab,
        ] {
            let (vk, _) = code(key).unwrap();
            if let Some(other) = seen.insert(vk, key) {
                panic!("{key:?} and {other:?} share vk {vk}");
            }
        }
    }
}
