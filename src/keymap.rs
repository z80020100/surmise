//! Which key does what inside the menu.
//!
//! A key the map binds is an action and never reaches the line. Every other
//! key goes to `crate::keys`, which edits the line the way the shell would.
//! The `[keys]` table in `config.toml` names an action by the name the other
//! engine's own settings give it and lists the keys that take it. An action
//! the file names loses its default keys to the ones the file lists. An
//! empty list leaves it unbound.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What a bound key does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Take the highlighted row. Enter's own meaning.
    InsertSelected,
    /// Take the prefix the rows share. Tab's own meaning.
    InsertCommonPrefix,
    /// The prefix the rows share, or the next row where they share none.
    InsertCommonPrefixOrNavigateDown,
    /// The prefix the rows share, or the highlighted row where they share
    /// none.
    InsertCommonPrefixOrInsertSelected,
    /// Take the highlighted row and run the line.
    InsertSelectedAndExecute,
    /// Leave the menu and keep the line.
    HideAutocomplete,
    NavigateUp,
    NavigateDown,
    /// Put the highlight on the row on screen that the number names. 1 is
    /// the first.
    SelectSuggestion(u8),
    /// Open the whole of the word under the list or close it again.
    ToggleDescription,
    /// Swap how what was typed reaches a row, for this menu alone.
    ToggleFuzzySearch,
    /// Take what the highlighted row adds at the end of the line, or move
    /// the cursor right.
    AcceptRight,
    /// Leave and put back the line the menu opened on.
    Cancel,
}

impl Action {
    /// Every action, in the order `settings show` lists them.
    const ALL: [Action; 22] = [
        Action::InsertSelected,
        Action::InsertCommonPrefix,
        Action::InsertCommonPrefixOrNavigateDown,
        Action::InsertCommonPrefixOrInsertSelected,
        Action::InsertSelectedAndExecute,
        Action::HideAutocomplete,
        Action::NavigateUp,
        Action::NavigateDown,
        Action::SelectSuggestion(1),
        Action::SelectSuggestion(2),
        Action::SelectSuggestion(3),
        Action::SelectSuggestion(4),
        Action::SelectSuggestion(5),
        Action::SelectSuggestion(6),
        Action::SelectSuggestion(7),
        Action::SelectSuggestion(8),
        Action::SelectSuggestion(9),
        Action::SelectSuggestion(10),
        Action::ToggleDescription,
        Action::ToggleFuzzySearch,
        Action::AcceptRight,
        Action::Cancel,
    ];

    /// The name the `[keys]` table gives the action.
    pub fn name(self) -> String {
        match self {
            Action::InsertSelected => "insertSelected".into(),
            Action::InsertCommonPrefix => "insertCommonPrefix".into(),
            Action::InsertCommonPrefixOrNavigateDown => "insertCommonPrefixOrNavigateDown".into(),
            Action::InsertCommonPrefixOrInsertSelected => {
                "insertCommonPrefixOrInsertSelected".into()
            }
            Action::InsertSelectedAndExecute => "insertSelectedAndExecute".into(),
            Action::HideAutocomplete => "hideAutocomplete".into(),
            Action::NavigateUp => "navigateUp".into(),
            Action::NavigateDown => "navigateDown".into(),
            Action::SelectSuggestion(n) => format!("selectSuggestion{n}"),
            Action::ToggleDescription => "toggleDescription".into(),
            Action::ToggleFuzzySearch => "toggleFuzzySearch".into(),
            Action::AcceptRight => "acceptRight".into(),
            Action::Cancel => "cancel".into(),
        }
    }

    fn named(name: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.name() == name)
    }

    /// The keys the action takes where the file says nothing of it.
    fn defaults(self) -> Vec<Key> {
        let keys: &[&str] = match self {
            Action::InsertSelected => &["enter"],
            Action::InsertCommonPrefix => &["tab"],
            Action::HideAutocomplete => &["esc"],
            Action::NavigateUp => &["up", "shift+tab", "ctrl+p"],
            Action::NavigateDown => &["down", "ctrl+n", "ctrl+j"],
            Action::SelectSuggestion(n) => {
                return vec![Key::char(char::from(b'0' + n % 10), KeyModifiers::ALT)];
            }
            Action::ToggleDescription => &["ctrl+o"],
            Action::AcceptRight => &["right"],
            Action::Cancel => &["ctrl+c", "ctrl+g"],
            Action::InsertCommonPrefixOrNavigateDown
            | Action::InsertCommonPrefixOrInsertSelected
            | Action::InsertSelectedAndExecute
            | Action::ToggleFuzzySearch => &[],
        };
        keys.iter().filter_map(|name| Key::parse(name)).collect()
    }
}

/// One key and the modifiers held with it, the way a binding names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    code: KeyCode,
    modifiers: KeyModifiers,
}

/// The keys a binding may name by a word rather than by the character they
/// type.
const NAMED: [(&str, KeyCode); 13] = [
    ("tab", KeyCode::Tab),
    ("enter", KeyCode::Enter),
    ("esc", KeyCode::Esc),
    ("up", KeyCode::Up),
    ("down", KeyCode::Down),
    ("left", KeyCode::Left),
    ("right", KeyCode::Right),
    ("home", KeyCode::Home),
    ("end", KeyCode::End),
    ("backspace", KeyCode::Backspace),
    ("delete", KeyCode::Delete),
    ("pageup", KeyCode::PageUp),
    ("pagedown", KeyCode::PageDown),
];

/// The other spellings a binding may use for a modifier or a key.
const ALIASES: [(&str, &str); 5] = [
    ("control", "ctrl"),
    ("option", "alt"),
    ("meta", "alt"),
    ("return", "enter"),
    ("escape", "esc"),
];

impl Key {
    fn char(c: char, modifiers: KeyModifiers) -> Key {
        Key {
            code: KeyCode::Char(c),
            modifiers,
        }
    }

    /// The key a binding spells as `ctrl+n`, `shift+tab` or `alt+1`. `None`
    /// for a spelling no key answers to.
    pub fn parse(spelled: &str) -> Option<Key> {
        // `+` itself is a key and `ctrl++` names it.
        let (held, last) = match spelled.strip_suffix("++") {
            Some(held) => (held, "+"),
            None => spelled.rsplit_once('+').unwrap_or(("", spelled)),
        };
        let mut modifiers = KeyModifiers::NONE;
        if !held.is_empty() {
            for part in held.split('+') {
                modifiers |= match alias(part) {
                    "ctrl" => KeyModifiers::CONTROL,
                    "alt" => KeyModifiers::ALT,
                    "shift" => KeyModifiers::SHIFT,
                    _ => return None,
                };
            }
        }
        let last = alias(last);
        let code = match NAMED.iter().find(|(name, _)| *name == last) {
            Some((_, code)) => *code,
            None if last == "space" => KeyCode::Char(' '),
            None => {
                let mut chars = last.chars();
                let (Some(c), None) = (chars.next(), chars.next()) else {
                    return None;
                };
                KeyCode::Char(c.to_ascii_lowercase())
            }
        };
        Some(Key { code, modifiers })
    }

    /// The key an event is, spelled the way a binding would spell it. A
    /// terminal reports Shift-Tab as a key of its own and a capital letter
    /// as the letter with Shift held. Shift is the character itself for
    /// everything else a person types.
    fn of(event: &KeyEvent) -> Key {
        let mut modifiers =
            event.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        let code = match event.code {
            KeyCode::BackTab => {
                modifiers |= KeyModifiers::SHIFT;
                KeyCode::Tab
            }
            KeyCode::Char(c) if c.is_ascii_uppercase() => {
                modifiers |= KeyModifiers::SHIFT;
                KeyCode::Char(c.to_ascii_lowercase())
            }
            KeyCode::Char(c) => {
                modifiers -= KeyModifiers::SHIFT;
                KeyCode::Char(c)
            }
            code => code,
        };
        Key { code, modifiers }
    }

    /// The spelling `settings show` writes back.
    fn spelled(self) -> String {
        let mut out = String::new();
        for (held, name) in [
            (KeyModifiers::CONTROL, "ctrl+"),
            (KeyModifiers::ALT, "alt+"),
            (KeyModifiers::SHIFT, "shift+"),
        ] {
            if self.modifiers.contains(held) {
                out.push_str(name);
            }
        }
        match self.code {
            KeyCode::Char(' ') => out.push_str("space"),
            KeyCode::Char(c) => out.push(c),
            code => out.push_str(
                NAMED
                    .iter()
                    .find(|(_, named)| *named == code)
                    .map_or("", |(name, _)| name),
            ),
        }
        out
    }

    /// Whether binding this key takes it away from editing the line: a
    /// character typed on its own, Backspace or Left.
    fn edits(self) -> bool {
        let plain = !self
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        match self.code {
            KeyCode::Char(_) => plain,
            KeyCode::Backspace | KeyCode::Left => true,
            _ => false,
        }
    }
}

fn alias(word: &str) -> &str {
    ALIASES
        .iter()
        .find(|(other, _)| *other == word)
        .map_or(word, |(_, canonical)| canonical)
}

/// Every action and the keys that take it.
#[derive(Clone, Debug, PartialEq)]
pub struct Keymap(Vec<(Action, Vec<Key>)>);

impl Default for Keymap {
    fn default() -> Self {
        Keymap(Action::ALL.into_iter().map(|a| (a, a.defaults())).collect())
    }
}

impl Keymap {
    /// The map a `[keys]` table asks for. An action the table names takes
    /// the keys it lists and no key it lists takes any other action. A name
    /// no action answers to, a value that is not a list of key names and a
    /// key no spelling reaches each earn a warning and change nothing. A
    /// binding that takes a key away from the line earns one and stands.
    pub fn from_table<'a>(
        table: impl IntoIterator<Item = (&'a String, &'a toml::Value)>,
        warnings: &mut Vec<String>,
    ) -> Keymap {
        let mut map = Keymap::default();
        for (name, value) in table {
            let Some(action) = Action::named(name) else {
                warnings.push(format!(
                    "keys has no action named {}",
                    crate::ui::printable(name)
                ));
                continue;
            };
            let Some(keys) = keys_of(value) else {
                warnings.push(format!("keys.{name} is not a list of key names"));
                continue;
            };
            let Some(keys) = keys
                .iter()
                .map(|k| Key::parse(k))
                .collect::<Option<Vec<_>>>()
            else {
                warnings.push(format!(
                    "keys.{name} names a key no spelling reaches: {}",
                    crate::ui::printable(&keys.join(", "))
                ));
                continue;
            };
            for key in keys.iter().filter(|key| key.edits()) {
                warnings.push(format!(
                    "keys.{name} takes {} away from editing the line",
                    key.spelled()
                ));
            }
            for (other, taken) in &mut map.0 {
                if *other == action {
                    taken.clone_from(&keys);
                } else {
                    taken.retain(|key| !keys.contains(key));
                }
            }
        }
        map
    }

    /// The action `event` takes, if a binding names it.
    pub fn action(&self, event: &KeyEvent) -> Option<Action> {
        let key = Key::of(event);
        self.0
            .iter()
            .find(|(_, keys)| keys.contains(&key))
            .map(|(action, _)| *action)
    }

    /// The table as `settings show` prints it.
    pub fn table(&self) -> toml_edit::Table {
        let mut table = toml_edit::Table::new();
        for (action, keys) in &self.0 {
            let spelled: toml_edit::Array = keys.iter().map(|k| k.spelled()).collect();
            table[&action.name()] = toml_edit::value(spelled);
        }
        table
    }
}

/// The key names a value lists. One name on its own counts as a list of one.
fn keys_of(value: &toml::Value) -> Option<Vec<String>> {
    match value {
        toml::Value::String(one) => Some(vec![one.clone()]),
        toml::Value::Array(many) => many
            .iter()
            .map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn map(text: &str) -> (Keymap, Vec<String>) {
        let table: toml::Table = text.parse().unwrap();
        let mut warnings = Vec::new();
        let map = Keymap::from_table(&table, &mut warnings);
        (map, warnings)
    }

    #[test]
    fn every_default_key_takes_the_action_it_always_took() {
        let map = Keymap::default();
        for (code, modifiers, action) in [
            (KeyCode::Enter, KeyModifiers::NONE, Action::InsertSelected),
            (KeyCode::Tab, KeyModifiers::NONE, Action::InsertCommonPrefix),
            (KeyCode::BackTab, KeyModifiers::SHIFT, Action::NavigateUp),
            (
                KeyCode::Char('p'),
                KeyModifiers::CONTROL,
                Action::NavigateUp,
            ),
            (
                KeyCode::Char('j'),
                KeyModifiers::CONTROL,
                Action::NavigateDown,
            ),
            (
                KeyCode::Char('3'),
                KeyModifiers::ALT,
                Action::SelectSuggestion(3),
            ),
            (
                KeyCode::Char('0'),
                KeyModifiers::ALT,
                Action::SelectSuggestion(10),
            ),
            (
                KeyCode::Char('o'),
                KeyModifiers::CONTROL,
                Action::ToggleDescription,
            ),
            (KeyCode::Char('g'), KeyModifiers::CONTROL, Action::Cancel),
            (KeyCode::Esc, KeyModifiers::NONE, Action::HideAutocomplete),
            (KeyCode::Right, KeyModifiers::NONE, Action::AcceptRight),
        ] {
            assert_eq!(
                map.action(&press(code, modifiers)),
                Some(action),
                "{code:?}"
            );
        }
        // A character, Ctrl-K and Left edit the line.
        for (code, modifiers) in [
            (KeyCode::Char('a'), KeyModifiers::NONE),
            (KeyCode::Char('A'), KeyModifiers::SHIFT),
            (KeyCode::Char('k'), KeyModifiers::CONTROL),
            (KeyCode::Left, KeyModifiers::NONE),
        ] {
            assert_eq!(map.action(&press(code, modifiers)), None, "{code:?}");
        }
    }

    #[test]
    fn a_table_rebinds_an_action_and_takes_its_keys_from_the_others() {
        let (map, warnings) = map("insertCommonPrefixOrNavigateDown = \"tab\"\n\
             toggleFuzzySearch = [\"ctrl+f\"]\n\
             cancel = []\n");
        assert!(warnings.is_empty(), "{warnings:?}");
        let tab = press(KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(
            map.action(&tab),
            Some(Action::InsertCommonPrefixOrNavigateDown)
        );
        let ctrl_f = press(KeyCode::Char('f'), KeyModifiers::CONTROL);
        assert_eq!(map.action(&ctrl_f), Some(Action::ToggleFuzzySearch));
        let ctrl_c = press(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(map.action(&ctrl_c), None);
    }

    #[test]
    fn a_table_that_names_nothing_real_earns_a_warning_and_changes_nothing() {
        let (map, warnings) = map("jump = \"ctrl+x\"\n\
             navigateDown = \"hyper+n\"\n\
             navigateUp = 3\n");
        assert_eq!(map, Keymap::default());
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(warnings[0].contains("no action named jump"), "{warnings:?}");
        assert!(warnings[1].contains("hyper+n"), "{warnings:?}");
        assert!(warnings[2].contains("not a list"), "{warnings:?}");
    }

    #[test]
    fn a_binding_that_takes_a_key_from_the_line_says_so_and_stands() {
        let (map, warnings) = map("navigateDown = [\"j\", \"backspace\"]\n");
        let j = press(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(map.action(&j), Some(Action::NavigateDown));
        assert_eq!(warnings.len(), 2, "{warnings:?}");
    }

    #[test]
    fn every_spelling_reads_back_as_itself() {
        for spelled in [
            "ctrl+n",
            "shift+tab",
            "alt+1",
            "esc",
            "space",
            "ctrl+alt+x",
            "ctrl++",
        ] {
            let key = Key::parse(spelled).expect(spelled);
            assert_eq!(Key::parse(&key.spelled()), Some(key), "{spelled}");
        }
        assert_eq!(Key::parse("control+p"), Key::parse("ctrl+p"));
        assert_eq!(Key::parse("escape"), Key::parse("esc"));
        assert!(Key::parse("ctrl+").is_none());
        assert!(Key::parse("ctrl+nope").is_none());
    }
}
