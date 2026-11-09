//! Terminal theme: three levels, chosen from the environment only.
//!
//! `Ascii` (`<*>`) is the safe floor; `Unicode` (`‹ ◆ ›`) needs a UTF-8 locale and a capable
//! terminal; `Color` adds the brand colors. `NO_COLOR`, `TERM=dumb`, an unset `TERM`, a non-UTF-8
//! locale and output that is not a terminal each pull the level down.

/// How much the terminal is trusted to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Plain ASCII, no escape sequences.
    Ascii,
    /// UTF-8 symbols, no escape sequences.
    Unicode,
    /// UTF-8 symbols and ANSI colors.
    Color,
}

/// What a piece of text is, for coloring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The brand mark and the prompt.
    Mark,
    /// A problem.
    Error,
    /// Secondary information.
    Note,
}

/// The chosen level, with the glyphs and colors that go with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    level: Level,
    truecolor: bool,
}

fn is_utf8_locale(env: &dyn Fn(&str) -> Option<String>) -> bool {
    // The first variable that is set and not empty decides, as in `setlocale`.
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .filter_map(|name| env(name))
        .find(|value| !value.is_empty())
        .is_some_and(|value| {
            let lower = value.to_ascii_lowercase();
            lower.contains("utf-8") || lower.contains("utf8")
        })
}

impl Theme {
    /// A theme at exactly `level` (for tests and for `--ascii` style switches).
    #[must_use]
    pub fn at(level: Level) -> Self {
        Self {
            level,
            truecolor: false,
        }
    }

    /// Choose from the environment. `is_tty` is whether the output is a terminal.
    #[must_use]
    pub fn detect(env: &dyn Fn(&str) -> Option<String>, is_tty: bool) -> Self {
        let term = env("TERM").unwrap_or_default();
        if !is_tty || term.is_empty() || term == "dumb" || !is_utf8_locale(env) {
            return Self::at(Level::Ascii);
        }
        // https://no-color.org: present and not empty.
        let no_color = env("NO_COLOR").is_some_and(|value| !value.is_empty());
        if no_color {
            return Self::at(Level::Unicode);
        }
        let colorterm = env("COLORTERM").unwrap_or_default();
        Self {
            level: Level::Color,
            truecolor: colorterm == "truecolor" || colorterm == "24bit",
        }
    }

    /// The level in use.
    #[must_use]
    pub fn level(&self) -> Level {
        self.level
    }

    /// The brand mark.
    #[must_use]
    pub fn mark(&self) -> &'static str {
        match self.level {
            Level::Ascii => "<*>",
            Level::Unicode | Level::Color => "\u{2039} \u{25c6} \u{203a}",
        }
    }

    /// The input prompt.
    #[must_use]
    pub fn prompt(&self) -> String {
        format!("{} ", self.paint(Role::Mark, self.mark()))
    }

    /// `text` in the color of `role`, or unchanged below the `Color` level.
    #[must_use]
    pub fn paint(&self, role: Role, text: &str) -> String {
        if self.level != Level::Color {
            return text.to_owned();
        }
        // Brand gold #C8A66A and copper #B87355; the basic palette is the fallback.
        let code = match (role, self.truecolor) {
            (Role::Mark, true) => "38;2;200;166;106",
            (Role::Mark, false) => "33",
            (Role::Error, true) => "38;2;184;115;85",
            (Role::Error, false) => "31",
            (Role::Note, _) => "2",
        };
        format!("\u{1b}[{code}m{text}\u{1b}[0m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    const FULL: &[(&str, &str)] = &[("TERM", "xterm-256color"), ("LANG", "es_CL.UTF-8")];

    #[test]
    fn a_capable_terminal_gets_color() {
        assert_eq!(Theme::detect(&env(FULL), true).level(), Level::Color);
    }

    #[test]
    fn each_degrading_condition_lowers_the_level() {
        assert_eq!(Theme::detect(&env(FULL), false).level(), Level::Ascii);
        assert_eq!(
            Theme::detect(&env(&[("TERM", "dumb"), ("LANG", "C.UTF-8")]), true).level(),
            Level::Ascii
        );
        assert_eq!(
            Theme::detect(&env(&[("LANG", "C.UTF-8")]), true).level(),
            Level::Ascii,
            "an unset TERM is not trusted"
        );
        assert_eq!(
            Theme::detect(&env(&[("TERM", "xterm"), ("LANG", "C")]), true).level(),
            Level::Ascii,
            "a non-UTF-8 locale"
        );
        assert_eq!(
            Theme::detect(
                &env(&[("TERM", "xterm"), ("LANG", "C.UTF-8"), ("NO_COLOR", "1")]),
                true
            )
            .level(),
            Level::Unicode
        );
    }

    #[test]
    fn an_empty_no_color_is_ignored_and_lc_all_beats_lang() {
        let empty = env(&[("TERM", "xterm"), ("LANG", "C.UTF-8"), ("NO_COLOR", "")]);
        assert_eq!(Theme::detect(&empty, true).level(), Level::Color);
        let override_lang = env(&[("TERM", "xterm"), ("LC_ALL", "C"), ("LANG", "C.UTF-8")]);
        assert_eq!(Theme::detect(&override_lang, true).level(), Level::Ascii);
    }

    #[test]
    fn only_the_color_level_emits_escape_sequences() {
        let text = "x";
        assert_eq!(Theme::at(Level::Ascii).paint(Role::Error, text), "x");
        assert_eq!(Theme::at(Level::Unicode).paint(Role::Mark, text), "x");
        assert!(
            Theme::at(Level::Color)
                .paint(Role::Mark, text)
                .contains('\u{1b}')
        );
        assert_eq!(Theme::at(Level::Ascii).mark(), "<*>");
        assert!(Theme::at(Level::Ascii).prompt().is_ascii());
        let truecolor = env(&[
            ("TERM", "xterm"),
            ("LANG", "C.UTF-8"),
            ("COLORTERM", "truecolor"),
        ]);
        assert!(
            Theme::detect(&truecolor, true)
                .paint(Role::Mark, "x")
                .contains("38;2;200;166;106")
        );
    }
}
