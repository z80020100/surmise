//! The line editing.
//!
//! Only the keys that change the line or move the highlight are here. Enter,
//! Tab and Escape answer for the menu as a whole rather than for the line and
//! the picker therefore holds those itself.

use crate::app::App;
use crate::ui;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A paste as line text. A newline would run the line and becomes a space.
/// Every other control character goes. One that stayed would show nothing
/// where it sits and would still reach the shell's own editor.
pub fn pasted(s: &str) -> String {
    ui::printable(&s.replace(['\n', '\r'], " "))
}

/// Apply one key. A key with no binding here leaves the state as it was.
///
/// The caller has already dropped a key release. One that reached here would
/// type the character a second time.
pub fn edit(app: &mut App, k: KeyEvent) {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let typed = k.modifiers.difference(KeyModifiers::SHIFT).is_empty();
    match k.code {
        KeyCode::Char('a') if ctrl => app.line.home(),
        KeyCode::Char('e') if ctrl => app.line.end(),
        KeyCode::Char('u') if ctrl => {
            app.line.kill_to_start();
            app.edited();
        }
        KeyCode::Char('k') if ctrl => {
            app.line.kill_to_end();
            app.edited();
        }
        KeyCode::Char('w') if ctrl => {
            app.line.kill_word_back();
            app.edited();
        }
        // Shift is part of typing a character. Every other modifier makes the
        // key a command and the character commands are the arms above. Alt
        // and a letter is a word motion in most editors and typing the letter
        // is never what the person meant by it.
        KeyCode::Char(c) if typed => {
            app.line.insert(c.encode_utf8(&mut [0; 4]));
            app.edited();
        }
        KeyCode::Backspace => {
            app.line.backspace();
            app.edited();
        }
        KeyCode::Delete => {
            app.line.delete();
            app.edited();
        }
        KeyCode::Left => app.line.left(),
        KeyCode::Home => app.line.home(),
        KeyCode::End => app.line.end(),
        KeyCode::Up | KeyCode::BackTab => app.step(-1),
        KeyCode::Down => app.step(1),
        KeyCode::Right => {
            // At the end of the line the right arrow takes the completion.
            // `adds_to_the_line` rather than the ghost. The ghost shows
            // nothing when the name corrects the case of what was typed.
            if app.line.at_end() && app.adds_to_the_line() {
                app.accept();
            } else {
                app.line.right();
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Fixture;

    fn press(app: &mut App, code: KeyCode) {
        press_with(app, code, KeyModifiers::NONE);
    }

    fn press_with(app: &mut App, code: KeyCode, m: KeyModifiers) {
        edit(app, KeyEvent::new(code, m));
    }

    #[test]
    fn a_typed_character_reaches_the_line() {
        let f = Fixture::new(&["work"]);
        let mut a = App::over(f.path(), "cd wor");
        press(&mut a, KeyCode::Char('k'));
        assert_eq!(a.line.text(), "cd work");
    }

    #[test]
    fn a_multibyte_character_reaches_the_line_whole() {
        let f = Fixture::new(&["work"]);
        let mut a = App::over(f.path(), "cd ");
        press(&mut a, KeyCode::Char('文'));
        assert_eq!(a.line.text(), "cd 文");
    }

    #[test]
    fn a_capital_letter_is_still_typing() {
        let f = Fixture::new(&["work"]);
        let mut a = App::over(f.path(), "cd ");
        press_with(&mut a, KeyCode::Char('W'), KeyModifiers::SHIFT);
        assert_eq!(a.line.text(), "cd W");
    }

    #[test]
    fn a_letter_under_control_is_not_typing() {
        let f = Fixture::new(&["work"]);
        let mut a = App::over(f.path(), "cd wor");
        press_with(&mut a, KeyCode::Char('x'), KeyModifiers::CONTROL);
        assert_eq!(a.line.text(), "cd wor");
    }

    #[test]
    fn a_letter_under_alt_is_not_typing() {
        let f = Fixture::new(&["work"]);
        let mut a = App::over(f.path(), "cd wor");
        press_with(&mut a, KeyCode::Char('d'), KeyModifiers::ALT);
        assert_eq!(a.line.text(), "cd wor");
    }

    #[test]
    fn control_a_and_control_e_go_to_the_ends() {
        let f = Fixture::new(&["work"]);
        let mut a = App::over(f.path(), "cd wor");
        press_with(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert_eq!(a.line.left_of_cursor(), "");
        press_with(&mut a, KeyCode::Char('e'), KeyModifiers::CONTROL);
        assert!(a.line.at_end());
    }

    #[test]
    fn home_and_end_go_to_the_ends() {
        let f = Fixture::new(&["work"]);
        let mut a = App::over(f.path(), "cd wor");
        press(&mut a, KeyCode::Home);
        assert_eq!(a.line.left_of_cursor(), "");
        press(&mut a, KeyCode::End);
        assert!(a.line.at_end());
    }

    #[test]
    fn control_u_kills_back_to_the_start() {
        let f = Fixture::new(&["work"]);
        let mut a = App::over(f.path(), "cd wor");
        press_with(&mut a, KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert_eq!(a.line.text(), "");
    }

    #[test]
    fn control_k_kills_to_the_end() {
        let f = Fixture::new(&["work"]);
        let mut a = App::over(f.path(), "cd wor");
        press(&mut a, KeyCode::Home);
        press_with(&mut a, KeyCode::Char('k'), KeyModifiers::CONTROL);
        assert_eq!(a.line.text(), "");
    }

    #[test]
    fn control_w_kills_the_word_behind() {
        let f = Fixture::new(&["work"]);
        let mut a = App::over(f.path(), "cd wor");
        press_with(&mut a, KeyCode::Char('w'), KeyModifiers::CONTROL);
        assert_eq!(a.line.text(), "cd ");
    }

    #[test]
    fn backspace_takes_the_character_behind() {
        let f = Fixture::new(&["work"]);
        let mut a = App::over(f.path(), "cd wor");
        press(&mut a, KeyCode::Backspace);
        assert_eq!(a.line.text(), "cd wo");
    }

    #[test]
    fn delete_takes_the_character_ahead() {
        let f = Fixture::new(&["work"]);
        let mut a = App::over(f.path(), "cd wor");
        press(&mut a, KeyCode::Left);
        press(&mut a, KeyCode::Delete);
        assert_eq!(a.line.text(), "cd wo");
    }

    #[test]
    fn the_arrows_move_inside_the_line() {
        let f = Fixture::new(&["work"]);
        let mut a = App::over(f.path(), "cd wor");
        press(&mut a, KeyCode::Left);
        assert_eq!(a.line.left_of_cursor(), "cd wo");
        press(&mut a, KeyCode::Right);
        assert!(a.line.at_end());
    }

    #[test]
    fn up_and_down_move_the_highlight() {
        let f = Fixture::new(&["work", "worse"]);
        let mut a = App::over(f.path(), "cd wor");
        assert_eq!(a.items.len(), 2);
        press(&mut a, KeyCode::Down);
        assert_eq!(a.selected, 1);
        press(&mut a, KeyCode::Down);
        assert_eq!(a.selected, 0);
        press(&mut a, KeyCode::Up);
        assert_eq!(a.selected, 1);
    }

    #[test]
    fn shift_tab_moves_the_highlight_back() {
        let f = Fixture::new(&["work", "worse"]);
        let mut a = App::over(f.path(), "cd wor");
        press(&mut a, KeyCode::BackTab);
        assert_eq!(a.selected, 1);
    }

    #[test]
    fn the_right_arrow_at_the_end_takes_the_completion() {
        let f = Fixture::new(&["work"]);
        let mut a = App::over(f.path(), "cd wor");
        assert_eq!(a.ghost(), "k/");
        press(&mut a, KeyCode::Right);
        assert_eq!(a.line.text(), "cd work/");
    }

    #[test]
    fn the_right_arrow_with_nothing_to_take_only_moves() {
        let f = Fixture::new(&["other"]);
        let mut a = App::over(f.path(), "cd wor");
        assert_eq!(a.ghost(), "");
        press(&mut a, KeyCode::Right);
        assert_eq!(a.line.text(), "cd wor");
    }

    #[test]
    fn typing_brings_a_dismissed_menu_back() {
        let f = Fixture::new(&["work"]);
        let mut a = App::over(f.path(), "cd wo");
        a.dismissed = true;
        press(&mut a, KeyCode::Char('r'));
        assert!(a.menu_open());
    }

    #[test]
    fn moving_the_cursor_leaves_a_dismissed_menu_shut() {
        let f = Fixture::new(&["work"]);
        let mut a = App::over(f.path(), "cd wor");
        a.dismissed = true;
        press(&mut a, KeyCode::Home);
        press(&mut a, KeyCode::End);
        assert!(!a.menu_open());
    }

    #[test]
    fn a_key_with_no_binding_changes_nothing() {
        let f = Fixture::new(&["work", "worse"]);
        let mut a = App::over(f.path(), "cd wor");
        press(&mut a, KeyCode::F(5));
        press(&mut a, KeyCode::PageDown);
        assert_eq!(a.line.text(), "cd wor");
        assert_eq!(a.selected, 0);
    }

    #[test]
    fn a_pasted_newline_becomes_a_space() {
        assert_eq!(pasted("a\nb\r\nc"), "a b  c");
    }

    #[test]
    fn a_pasted_control_character_never_reaches_the_line() {
        assert_eq!(pasted("a\x1b[31mb"), "a[31mb");
        assert_eq!(pasted("a\x07b\x7fc"), "abc");
    }

    #[test]
    fn ordinary_text_comes_through_a_paste_whole() {
        assert_eq!(pasted("cd '日 本'/"), "cd '日 本'/");
    }
}
