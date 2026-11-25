//! The personality-file format of SPEC.md section 6.5.
//!
//! A file is UTF-8 text with one `key=value` per line. The parser is bounded
//! (file size, line count and line length) and every error carries the line
//! number it was found on.

use std::fmt;

use super::{Difficulty, Personality, Setting};
use crate::knobs::{knob_info, KnobError, KnobKind};

/// Largest file the parser accepts, in bytes.
pub const MAX_FILE_BYTES: usize = 64 * 1024;
/// Most lines the parser accepts.
pub const MAX_LINES: usize = 2_000;
/// Longest line the parser accepts, in bytes.
pub const MAX_LINE_BYTES: usize = 1_024;

/// Why a line (or the whole file) was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseErrorKind {
    /// The file is longer than [`MAX_FILE_BYTES`]. Reported on line 0.
    FileTooLarge {
        /// The size found.
        bytes: usize,
    },
    /// The file has more than [`MAX_LINES`] lines. Reported on the first
    /// line past the limit.
    TooManyLines,
    /// A line is longer than [`MAX_LINE_BYTES`].
    LineTooLong {
        /// The length found.
        bytes: usize,
    },
    /// A line that is not blank or a comment has no `=`.
    MissingEquals,
    /// A line starts with `=`.
    EmptyKey,
    /// The key is neither a reserved key nor a knob.
    UnknownKnob(String),
    /// The knob exists but personality files may not set it.
    NotExposed(String),
    /// The value does not parse as the knob's type.
    BadValue {
        /// The knob.
        knob: String,
        /// The offending text.
        text: String,
        /// The knob's type.
        expected: KnobKind,
    },
    /// `difficulty=` names no known difficulty.
    BadDifficulty(String),
    /// The file names another engine, which may not set knobs.
    KnobsForOtherEngine {
        /// The engine named.
        engine: String,
    },
}

/// A parse failure with the line it was found on (1-based; 0 means the whole
/// file).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// The 1-based line number.
    pub line: usize,
    /// What went wrong.
    pub kind: ParseErrorKind,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(f, "personality file: ")?;
        } else {
            write!(f, "line {}: ", self.line)?;
        }
        match &self.kind {
            ParseErrorKind::FileTooLarge { bytes } => {
                write!(f, "file is {bytes} bytes, over the {MAX_FILE_BYTES} limit")
            }
            ParseErrorKind::TooManyLines => write!(f, "more than {MAX_LINES} lines"),
            ParseErrorKind::LineTooLong { bytes } => {
                write!(f, "line is {bytes} bytes, over the {MAX_LINE_BYTES} limit")
            }
            ParseErrorKind::MissingEquals => write!(f, "expected `key=value`"),
            ParseErrorKind::EmptyKey => write!(f, "empty key"),
            ParseErrorKind::UnknownKnob(k) => write!(f, "unknown knob `{k}`"),
            ParseErrorKind::NotExposed(k) => {
                write!(f, "knob `{k}` is not exposed to personality files")
            }
            ParseErrorKind::BadValue {
                knob,
                text,
                expected,
            } => write!(f, "`{text}` is not a valid {expected} for knob `{knob}`"),
            ParseErrorKind::BadDifficulty(d) => write!(f, "unknown difficulty `{d}`"),
            ParseErrorKind::KnobsForOtherEngine { engine } => {
                write!(f, "a file for engine `{engine}` may not set knobs")
            }
        }
    }
}

impl std::error::Error for ParseError {}

fn err(line: usize, kind: ParseErrorKind) -> ParseError {
    ParseError { line, kind }
}

/// Parses a personality file.
///
/// `default_name` is the file name without its extension; it names the
/// personality unless the file has a `name=` line.
pub fn parse_personality(text: &str, default_name: &str) -> Result<Personality, ParseError> {
    if text.len() > MAX_FILE_BYTES {
        return Err(err(0, ParseErrorKind::FileTooLarge { bytes: text.len() }));
    }
    let mut p = Personality::new(default_name);
    let mut first_knob_line: Option<usize> = None;
    for (index, raw) in text.lines().enumerate() {
        let line_no = index + 1;
        if line_no > MAX_LINES {
            return Err(err(line_no, ParseErrorKind::TooManyLines));
        }
        if raw.len() > MAX_LINE_BYTES {
            return Err(err(
                line_no,
                ParseErrorKind::LineTooLong { bytes: raw.len() },
            ));
        }
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(err(line_no, ParseErrorKind::MissingEquals));
        };
        let (key, value) = (key.trim(), value.trim());
        if key.is_empty() {
            return Err(err(line_no, ParseErrorKind::EmptyKey));
        }
        if key.eq_ignore_ascii_case("name") {
            p.name = value.to_string();
        } else if key.eq_ignore_ascii_case("description") {
            p.description = value.to_string();
        } else if key.eq_ignore_ascii_case("difficulty") {
            let d = Difficulty::from_name(value)
                .ok_or_else(|| err(line_no, ParseErrorKind::BadDifficulty(value.to_string())))?;
            p.difficulty = Some(d);
        } else if key.eq_ignore_ascii_case("engine") {
            p.engine = Some(value.to_string());
        } else {
            let info = knob_info(key)
                .ok_or_else(|| err(line_no, ParseErrorKind::UnknownKnob(key.to_string())))?;
            if !info.exposed {
                return Err(err(line_no, ParseErrorKind::NotExposed(key.to_string())));
            }
            let parsed = info.parse_value(value).map_err(|e| match e {
                KnobError::BadValue {
                    name,
                    text,
                    expected,
                } => err(
                    line_no,
                    ParseErrorKind::BadValue {
                        knob: name,
                        text,
                        expected,
                    },
                ),
                _ => err(line_no, ParseErrorKind::UnknownKnob(key.to_string())),
            })?;
            first_knob_line.get_or_insert(line_no);
            p.settings.push(Setting {
                knob: info.name,
                value: parsed,
            });
        }
    }
    if let (Some(line), Some(engine)) = (first_knob_line, p.engine.as_deref()) {
        if !p.is_own_engine() {
            return Err(err(
                line,
                ParseErrorKind::KnobsForOtherEngine {
                    engine: engine.to_string(),
                },
            ));
        }
    }
    Ok(p)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::knobs::{Fixed, Value};

    #[test]
    fn spec_example_parses() {
        let text = "# A raider that never builds towers.\nname=Raider\n\
                    description=Raids from the first minute.\ndifficulty=standard\n\
                    army.wave_size=4\ndef.towers=0\n";
        let p = parse_personality(text, "file").unwrap();
        assert_eq!(p.name, "Raider");
        assert_eq!(p.description, "Raids from the first minute.");
        assert_eq!(p.difficulty, Some(Difficulty::Standard));
        assert_eq!(p.engine, None);
        assert_eq!(
            p.settings,
            vec![
                Setting {
                    knob: "army.wave_size",
                    value: Value::Int(4)
                },
                Setting {
                    knob: "def.towers",
                    value: Value::Int(0)
                },
            ]
        );
    }

    #[test]
    fn default_name_whitespace_comments_and_case() {
        let text = " ; comment\n\n  NAME =  Spaced Out \n Difficulty = BRUTAL\n\
                    cheat.income_factor = 1.25 \n fort.every_tower = YES\n";
        let p = parse_personality(text, "fallback").unwrap();
        assert_eq!(p.name, "Spaced Out");
        assert_eq!(p.difficulty, Some(Difficulty::Brutal));
        assert_eq!(p.settings[0].value, Value::Num(Fixed::from_raw(12_500)));
        assert_eq!(p.settings[1].value, Value::Bool(true));
        let q = parse_personality("def.towers=1", "fallback").unwrap();
        assert_eq!(q.name, "fallback");
    }

    #[test]
    fn difficulty_aliases() {
        for (text, want) in [
            ("idle", Difficulty::Idle),
            ("none", Difficulty::Idle),
            ("off", Difficulty::Idle),
            ("easy", Difficulty::Easy),
            ("standard", Difficulty::Standard),
            ("medium", Difficulty::Standard),
            ("normal", Difficulty::Standard),
            ("hard", Difficulty::Hard),
            ("brutal", Difficulty::Brutal),
        ] {
            let p = parse_personality(&format!("difficulty={text}"), "x").unwrap();
            assert_eq!(p.difficulty, Some(want), "{text}");
        }
    }

    #[test]
    fn errors_carry_line_numbers() {
        let e = parse_personality("name=x\n\nbogus line\n", "x").unwrap_err();
        assert_eq!(e.line, 3);
        assert_eq!(e.kind, ParseErrorKind::MissingEquals);

        let e = parse_personality("\n\narmy.wave_sise=4", "x").unwrap_err();
        assert_eq!(e.line, 3);
        assert_eq!(e.kind, ParseErrorKind::UnknownKnob("army.wave_sise".into()));

        let e = parse_personality("army.wave_size=big", "x").unwrap_err();
        assert_eq!(e.line, 1);
        assert!(matches!(e.kind, ParseErrorKind::BadValue { .. }));
        assert!(e.to_string().starts_with("line 1: "));

        let e = parse_personality("name=a\n=c", "x").unwrap_err();
        assert_eq!((e.line, e.kind), (2, ParseErrorKind::EmptyKey));

        let e = parse_personality("difficulty=godlike", "x").unwrap_err();
        assert_eq!(e.kind, ParseErrorKind::BadDifficulty("godlike".into()));

        let e = parse_personality("scout.enabled=0", "x").unwrap_err();
        assert_eq!(e.kind, ParseErrorKind::NotExposed("scout.enabled".into()));
    }

    #[test]
    fn other_engines_may_not_set_knobs() {
        let ok = parse_personality("engine=classic\nname=Theirs", "x").unwrap();
        assert_eq!(ok.engine.as_deref(), Some("classic"));
        assert!(!ok.is_own_engine());
        let e = parse_personality("def.towers=1\nengine=classic", "x").unwrap_err();
        assert_eq!(e.line, 1);
        assert!(matches!(e.kind, ParseErrorKind::KnobsForOtherEngine { .. }));
        let own = parse_personality("engine=BTM-AI\ndef.towers=1", "x").unwrap();
        assert!(own.is_own_engine());
    }

    #[test]
    fn bounds_are_enforced() {
        let big = "#".repeat(MAX_FILE_BYTES + 1);
        let e = parse_personality(&big, "x").unwrap_err();
        assert_eq!(e.line, 0);
        assert!(matches!(e.kind, ParseErrorKind::FileTooLarge { .. }));

        let long = format!("name={}", "a".repeat(MAX_LINE_BYTES));
        let e = parse_personality(&long, "x").unwrap_err();
        assert!(matches!(e.kind, ParseErrorKind::LineTooLong { .. }));

        let many = "#\n".repeat(MAX_LINES + 1);
        let e = parse_personality(&many, "x").unwrap_err();
        assert_eq!(e.line, MAX_LINES + 1);
        assert_eq!(e.kind, ParseErrorKind::TooManyLines);
        assert!(parse_personality(&"#\n".repeat(MAX_LINES), "x").is_ok());
    }
}
