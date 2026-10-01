//! The menu's colours.
//!
//! Eight roles carry a colour each, named the way the other engine's own
//! themes name them. The `[theme]` table in `config.toml` gives any of them
//! another one. A colour is a 256-colour index or `#rrggbb`, and `rgb(r,g,b)`
//! reads the same as the second. The defaults are the palette `crate::ui`
//! writes down beside each one's reason.

/// One colour a terminal can draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colour {
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl Colour {
    /// The colour a theme value names. An integer is an index. A string is
    /// `#rrggbb`, `#rrggbbaa` with the alpha laid onto each channel the way
    /// the other engine lays it, or `rgb(r,g,b)`. `None` for anything else.
    pub fn parse(value: &toml::Value) -> Option<Colour> {
        match value {
            toml::Value::Integer(n) => u8::try_from(*n).ok().map(Colour::Indexed),
            toml::Value::String(s) => Colour::parse_text(s),
            _ => None,
        }
    }

    pub fn parse_text(text: &str) -> Option<Colour> {
        let text = text.trim();
        if let Some(hex) = text.strip_prefix('#') {
            let channel = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
            let alpha = match hex.len() {
                6 => 255,
                8 => channel(6)?,
                _ => return None,
            };
            let lay = |c: u8| (u16::from(c) * u16::from(alpha) / 255) as u8;
            return Some(Colour::Rgb(
                lay(channel(0)?),
                lay(channel(2)?),
                lay(channel(4)?),
            ));
        }
        let inside = text.strip_prefix("rgb(")?.strip_suffix(')')?;
        let channels: Vec<u8> = inside
            .split(',')
            .map(|c| c.trim().parse().ok())
            .collect::<Option<_>>()?;
        match channels[..] {
            [r, g, b] => Some(Colour::Rgb(r, g, b)),
            _ => None,
        }
    }

    /// The escape that draws this colour as a ground or as a character.
    fn escape(self, ground: Ground) -> String {
        let layer = match ground {
            Ground::Back => 48,
            Ground::Fore => 38,
        };
        match self {
            Colour::Indexed(n) => format!("\x1b[{layer};5;{n}m"),
            Colour::Rgb(r, g, b) => format!("\x1b[{layer};2;{r};{g};{b}m"),
        }
    }

    /// The value `settings show` writes back.
    fn value(self) -> toml_edit::Value {
        match self {
            Colour::Indexed(n) => i64::from(n).into(),
            Colour::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}").into(),
        }
    }
}

/// Whether a role colours a ground or the characters drawn on one.
#[derive(Clone, Copy)]
enum Ground {
    Back,
    Fore,
}

/// Every role, its key in the table, what it colours and its default.
const ROLES: [(&str, Ground, Colour); 8] = [
    // The panel sits on a ground of its own that is a shade off the
    // terminal's.
    ("background", Ground::Back, Colour::Indexed(236)),
    ("text", Ground::Fore, Colour::Indexed(249)),
    // The ground under a character what was typed reached. A dark olive
    // rather than a tint of the panel's own grey. A mark then reads as a mark
    // rather than as another row.
    ("match_background", Ground::Back, Colour::Indexed(58)),
    // The word that says what the highlighted row is. It reads at the weight
    // a name does and italic alone is what sets it apart. What the row says
    // is worth reading rather than worth fading out.
    ("description_text", Ground::Fore, Colour::Indexed(249)),
    // The line that closes a panel and the one that separates the list from
    // what the panel puts under it. A shade off the panel's own ground rather
    // than a name's colour: it separates two things rather than saying
    // anything of its own.
    ("description_border", Ground::Fore, Colour::Indexed(238)),
    ("selected_background", Ground::Back, Colour::Indexed(25)),
    // The name on the highlighted row's own ground. The glyph in front of it
    // wears a colour of its own and this is what puts the name back.
    ("selected_text", Ground::Fore, Colour::Indexed(15)),
    // The same mark on the highlighted row. That row's ground is a blue the
    // olive disappears into and a lighter tint of that blue takes over.
    (
        "selected_match_background",
        Ground::Back,
        Colour::Indexed(67),
    ),
];

/// The table's own names for its roles, in the order `ROLES` holds them.
pub const NAMES: [&str; 8] = [
    ROLES[0].0, ROLES[1].0, ROLES[2].0, ROLES[3].0, ROLES[4].0, ROLES[5].0, ROLES[6].0, ROLES[7].0,
];

/// Every role's colour and the escape that draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    colours: [Colour; 8],
    escapes: [String; 8],
}

impl Default for Theme {
    fn default() -> Self {
        Theme::of(ROLES.map(|(_, _, colour)| colour))
    }
}

impl Theme {
    fn of(colours: [Colour; 8]) -> Theme {
        let escapes = std::array::from_fn(|at| colours[at].escape(ROLES[at].1));
        Theme { colours, escapes }
    }

    /// The theme a `[theme]` table asks for. A key no role answers to and a
    /// value no colour reads as each earn a warning and leave that role on
    /// its default.
    pub fn from_table<'a>(
        table: impl IntoIterator<Item = (&'a String, &'a toml::Value)>,
        warnings: &mut Vec<String>,
    ) -> Theme {
        let mut colours = ROLES.map(|(_, _, colour)| colour);
        for (key, value) in table {
            let Some(at) = NAMES.iter().position(|name| name == key) else {
                warnings.push(format!(
                    "theme has no colour named {}",
                    crate::ui::printable(key)
                ));
                continue;
            };
            match Colour::parse(value) {
                Some(colour) => colours[at] = colour,
                None => warnings.push(format!(
                    "theme.{key} is not a colour: {}",
                    crate::ui::printable(&value.to_string())
                )),
            }
        }
        Theme::of(colours)
    }

    /// The table as `settings show` prints it.
    pub fn table(&self) -> toml_edit::Table {
        let mut table = toml_edit::Table::new();
        for (name, colour) in NAMES.iter().zip(self.colours) {
            table[name] = toml_edit::value(colour.value());
        }
        table
    }

    pub fn background(&self) -> &str {
        &self.escapes[0]
    }

    pub fn text(&self) -> &str {
        &self.escapes[1]
    }

    pub fn match_background(&self) -> &str {
        &self.escapes[2]
    }

    pub fn description_text(&self) -> &str {
        &self.escapes[3]
    }

    pub fn description_border(&self) -> &str {
        &self.escapes[4]
    }

    pub fn selected_background(&self) -> &str {
        &self.escapes[5]
    }

    pub fn selected_text(&self) -> &str {
        &self.escapes[6]
    }

    pub fn selected_match_background(&self) -> &str {
        &self.escapes[7]
    }
}

/// The `[theme]` table a theme file the other engine reads asks for. That
/// file is JSON in one of two shapes. A `version: "1.0"` one names the eight
/// colours under `theme`. A base16 one names `shade0` to `shade7` and
/// `accent0` to `accent7` and the eight come from those the way that engine
/// takes them, with its own dark shades for any the file leaves out. A
/// colour the file leaves out of the first shape is left out of the table
/// and the default stands for it.
pub fn import(json: &str) -> Result<toml_edit::Table, String> {
    let file: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("not a theme file: {e}"))?;
    let at = |path: &[&str]| -> Option<String> {
        path.iter()
            .try_fold(&file, |v, key| v.get(key))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    let picked: [Option<String>; 8] = if file.get("version").and_then(|v| v.as_str()) == Some("1.0")
        && file.get("theme").is_some()
    {
        [
            at(&["theme", "backgroundColor"]),
            at(&["theme", "textColor"]),
            at(&["theme", "matchBackgroundColor"]),
            at(&["theme", "description", "textColor"]),
            at(&["theme", "description", "borderColor"]),
            at(&["theme", "selection", "backgroundColor"]),
            at(&["theme", "selection", "textColor"]),
            at(&["theme", "selection", "matchBackgroundColor"]),
        ]
    } else {
        // A file that names no shade at all is no theme and the fillers alone
        // would write one nobody chose.
        let named = (0..8).any(|n| {
            at(&[&format!("shade{n}")]).is_some() || at(&[&format!("accent{n}")]).is_some()
        });
        if !named {
            return Err("the file names none of the colours a theme holds".to_string());
        }
        let shade = |name: &str, filler: &str| Some(at(&[name]).unwrap_or_else(|| filler.into()));
        [
            shade("shade0", "#181818"),
            shade("shade6", "#e8e8e8"),
            shade("shade1", "#282828"),
            shade("shade5", "#d8d8d8"),
            shade("shade1", "#282828"),
            shade("shade2", "#383838"),
            shade("shade7", "#f8f8f8"),
            shade("accent6", "#ba8baf"),
        ]
    };
    let mut table = toml_edit::Table::new();
    for (name, text) in NAMES.iter().zip(picked) {
        let Some(text) = text else {
            continue;
        };
        let colour = Colour::parse_text(&text)
            .ok_or_else(|| format!("{name} is not a colour: {}", crate::ui::printable(&text)))?;
        table[name] = toml_edit::value(colour.value());
    }
    if table.is_empty() {
        return Err("the file names none of the colours a theme holds".to_string());
    }
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme(text: &str) -> (Theme, Vec<String>) {
        let table: toml::Table = text.parse().unwrap();
        let mut warnings = Vec::new();
        (Theme::from_table(&table, &mut warnings), warnings)
    }

    #[test]
    fn every_way_of_naming_a_colour_reads() {
        let rgb = Some(Colour::Rgb(0x12, 0x34, 0x56));
        assert_eq!(Colour::parse_text("#123456"), rgb);
        assert_eq!(Colour::parse_text("rgb(18, 52, 86)"), rgb);
        assert_eq!(
            Colour::parse_text("#ffffff80"),
            Some(Colour::Rgb(128, 128, 128))
        );
        for bad in ["#12345", "rgb(1,2)", "blue", "#gggggg"] {
            assert_eq!(Colour::parse_text(bad), None, "{bad}");
        }
        assert_eq!(
            Colour::parse(&toml::Value::Integer(236)),
            Some(Colour::Indexed(236))
        );
        assert_eq!(Colour::parse(&toml::Value::Integer(256)), None);
    }

    #[test]
    fn a_table_colours_the_roles_it_names_and_leaves_the_rest() {
        let (theme, warnings) = theme("background = \"#102030\"\nselected_text = 231\n");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(theme.background(), "\x1b[48;2;16;32;48m");
        assert_eq!(theme.selected_text(), "\x1b[38;5;231m");
        assert_eq!(theme.text(), Theme::default().text());
    }

    #[test]
    fn a_name_or_a_value_nothing_reads_earns_a_warning_and_changes_nothing() {
        let (theme, warnings) = theme("ground = 1\ntext = \"blue\"\n");
        assert_eq!(theme, Theme::default());
        assert_eq!(warnings.len(), 2, "{warnings:?}");
    }

    #[test]
    fn a_version_one_file_names_its_colours_and_leaves_the_rest_out() {
        let table = import(
            r##"{"version": "1.0", "theme": {
                "backgroundColor": "rgb(48,48,48)", "textColor": "#b4b4b4",
                "matchBackgroundColor": "#5f5938",
                "selection": {"textColor": "#fdfdfd", "backgroundColor": "#1e5ac7"},
                "description": {"textColor": "#b4b4b4", "borderColor": "#414141"}}}"##,
        )
        .unwrap();
        assert_eq!(table["background"].as_str(), Some("#303030"));
        assert_eq!(table["selected_background"].as_str(), Some("#1e5ac7"));
        assert!(!table.contains_key("selected_match_background"));
    }

    #[test]
    fn a_base16_file_reads_through_its_shades() {
        let table = import(r##"{"shade0": "#101010", "shade6": "#e0e0e0", "accent6": "#aa00aa"}"##)
            .unwrap();
        assert_eq!(table["background"].as_str(), Some("#101010"));
        assert_eq!(table["text"].as_str(), Some("#e0e0e0"));
        assert_eq!(table["selected_match_background"].as_str(), Some("#aa00aa"));
        // A shade the file leaves out is the dark one the other engine fills.
        assert_eq!(table["selected_background"].as_str(), Some("#383838"));
    }

    #[test]
    fn a_file_that_is_no_theme_is_refused() {
        assert!(import("not json").is_err());
        assert!(import(r#"{"version": "1.0", "theme": {"textColor": "blue"}}"#).is_err());
        assert!(import(r#"{"version": "1.0", "theme": {}}"#).is_err());
        assert!(import(r#"{"name": "sample"}"#).is_err());
    }

    #[test]
    fn the_defaults_are_the_palette_the_menu_draws_with() {
        let t = Theme::default();
        assert_eq!(t.background(), crate::ui::PANEL);
        assert_eq!(t.text(), crate::ui::NAME);
        assert_eq!(t.match_background(), crate::ui::MARK);
        assert_eq!(t.description_text(), crate::ui::FOOT);
        assert_eq!(t.description_border(), crate::ui::BORDER);
        assert_eq!(t.selected_background(), crate::ui::PANEL_CHOSEN);
        assert_eq!(t.selected_text(), crate::ui::NAME_CHOSEN);
        assert_eq!(t.selected_match_background(), crate::ui::MARK_CHOSEN);
    }
}
