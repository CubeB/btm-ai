//! The built-in personalities of SPEC.md section 6.4 and the registry that
//! holds them together with user files (section 6.5, "Loading").
//!
//! The built-ins are written as personality-file text and go through the same
//! parser as a user's file, so a built-in cannot set a knob a file could not.

use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use super::file::{parse_personality, ParseError, MAX_FILE_BYTES};
use super::Personality;

/// Most `*.txt` files [`Registry::load_dir`] reads from one folder.
pub const MAX_FILES: usize = 256;

/// (file stem, text) of every built-in, in menu order.
const BUILTIN_SOURCES: &[(&str, &str)] = &[
    (
        "Balanced",
        "name=Balanced\n\
         description=The tuned default: no knobs of its own.\n",
    ),
    (
        "Rush",
        "name=Rush\n\
         description=Few towers, a small first wave and raider kbots.\n\
         open.mex=2\n\
         open.solar=2\n\
         def.towers=0\n\
         def.heavy_towers=0\n\
         def.outpost_towers=1\n\
         def.outpost_income_step=0\n\
         fort.where_attacked=off\n\
         army.wave_size=5\n\
         army.spent_size=2\n\
         army.reinforce=2\n\
         mix.lab_raider=4\n\
         mix.lab_rocket=1\n\
         mix.lab_artillery=0\n\
         army.raids=on\n\
         army.raid_size=3\n\
         tech.enabled=off\n",
    ),
    (
        "Turtle",
        "name=Turtle\n\
         description=Many walled towers and a large first wave.\n\
         def.towers=6\n\
         def.heavy_towers=4\n\
         def.outpost_towers=5\n\
         def.outpost_income_step=5\n\
         def.outpost_max=12\n\
         def.outpost_min_mex=1\n\
         fort.every_tower=on\n\
         def.base_aa_towers=2\n\
         army.wave_size=20\n\
         army.spent_size=6\n\
         army.raids=off\n",
    ),
    (
        "Tech Rush",
        "name=Tech Rush\n\
         description=Goes to tier two early.\n\
         def.towers=1\n\
         army.wave_size=12\n\
         mix.lab_raider=1\n\
         tech.enabled=on\n\
         tech.min_income=6\n\
         tech.min_value_ratio=0\n\
         eco.save_tech_s=480\n\
         build.adv_constructors=2\n",
    ),
    (
        "Easy",
        "name=Easy\n\
         description=Easy difficulty with the default personality.\n\
         difficulty=easy\n",
    ),
    (
        "Medium",
        "name=Medium\n\
         description=Standard difficulty with the default personality.\n\
         difficulty=standard\n",
    ),
    (
        "Hard",
        "name=Hard\n\
         description=Hard difficulty with the default personality.\n\
         difficulty=hard\n",
    ),
];

/// A failure to load one personality file from disk.
#[derive(Debug)]
pub enum LoadError {
    /// The file could not be read.
    Io(std::io::Error),
    /// The file is not valid UTF-8.
    NotUtf8,
    /// The file parsed badly.
    Parse(ParseError),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "cannot read: {e}"),
            LoadError::NotUtf8 => write!(f, "not valid UTF-8"),
            LoadError::Parse(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// What [`Registry::load_dir`] found.
#[derive(Debug, Default)]
pub struct LoadReport {
    /// Files that parsed and were added or replaced a personality.
    pub loaded: usize,
    /// Files that were skipped, with the reason (the caller logs these).
    pub skipped: Vec<(PathBuf, LoadError)>,
}

/// The personalities available to a lobby, looked up case-insensitively by
/// name.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    items: Vec<Personality>,
}

impl Registry {
    /// A registry holding only the built-ins: Balanced, Rush, Turtle, Tech
    /// Rush, Easy, Medium and Hard.
    pub fn builtin() -> Registry {
        let mut reg = Registry::default();
        for (stem, text) in BUILTIN_SOURCES {
            // The built-ins are covered by tests; a failure here would only
            // drop that entry rather than abort a game.
            if let Ok(p) = parse_personality(text, stem) {
                reg.insert(p);
            }
        }
        reg
    }

    /// Adds a personality, replacing any whose name matches case-insensitively
    /// (the replacement keeps the original's position).
    pub fn insert(&mut self, p: Personality) {
        match self
            .items
            .iter_mut()
            .find(|q| q.name.eq_ignore_ascii_case(&p.name))
        {
            Some(slot) => *slot = p,
            None => self.items.push(p),
        }
    }

    /// Looks a personality up by name, ignoring case.
    pub fn get(&self, name: &str) -> Option<&Personality> {
        self.items
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// All personalities, in menu order.
    pub fn all(&self) -> &[Personality] {
        &self.items
    }

    /// Parses `text` as the file called `file_stem` and inserts it.
    pub fn add_file_text(&mut self, file_stem: &str, text: &str) -> Result<(), ParseError> {
        self.insert(parse_personality(text, file_stem)?);
        Ok(())
    }

    /// Reads every `*.txt` file in `dir`, in file-name order, at most
    /// [`MAX_FILES`] of them. A file that fails is skipped and reported.
    /// Returns `Err` only if the folder itself cannot be listed.
    pub fn load_dir(&mut self, dir: &Path) -> std::io::Result<LoadReport> {
        let mut paths: Vec<PathBuf> = fs::read_dir(dir)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("txt")))
            .collect();
        paths.sort();
        paths.truncate(MAX_FILES);
        let mut report = LoadReport::default();
        for path in paths {
            match read_file(&path) {
                Ok(text) => {
                    let stem = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default();
                    match self.add_file_text(stem, &text) {
                        Ok(()) => report.loaded += 1,
                        Err(e) => report.skipped.push((path, LoadError::Parse(e))),
                    }
                }
                Err(e) => report.skipped.push((path, e)),
            }
        }
        Ok(report)
    }
}

/// Reads a file, refusing to read more than the parser would accept.
fn read_file(path: &Path) -> Result<String, LoadError> {
    let file = fs::File::open(path).map_err(LoadError::Io)?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(LoadError::Io)?;
    String::from_utf8(bytes).map_err(|_| LoadError::NotUtf8)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::knobs::Value;
    use crate::personality::Difficulty;

    #[test]
    fn all_builtins_parse() {
        for (stem, text) in BUILTIN_SOURCES {
            parse_personality(text, stem).unwrap_or_else(|e| panic!("{stem}: {e}"));
        }
        let reg = Registry::builtin();
        let names: Vec<_> = reg.all().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Balanced",
                "Rush",
                "Turtle",
                "Tech Rush",
                "Easy",
                "Medium",
                "Hard"
            ]
        );
    }

    #[test]
    fn lookup_is_case_insensitive_and_tiers_name_a_difficulty() {
        let reg = Registry::builtin();
        assert_eq!(
            reg.get("tech rush").map(|p| p.name.as_str()),
            Some("Tech Rush")
        );
        assert_eq!(
            reg.get("EASY").and_then(|p| p.difficulty),
            Some(Difficulty::Easy)
        );
        assert_eq!(
            reg.get("medium").and_then(|p| p.difficulty),
            Some(Difficulty::Standard)
        );
        assert_eq!(
            reg.get("hard").and_then(|p| p.difficulty),
            Some(Difficulty::Hard)
        );
        assert_eq!(reg.get("balanced").and_then(|p| p.difficulty), None);
        assert!(reg.get("balanced").is_some_and(|p| p.settings.is_empty()));
        assert!(reg.get("nobody").is_none());
    }

    #[test]
    fn rush_row_matches_the_table() {
        let reg = Registry::builtin();
        let rush = reg.get("Rush").unwrap();
        let get = |k: &str| rush.settings.iter().find(|s| s.knob == k).map(|s| s.value);
        assert_eq!(get("open.mex"), Some(Value::Int(2)));
        assert_eq!(get("army.wave_size"), Some(Value::Int(5)));
        assert_eq!(get("tech.enabled"), Some(Value::Bool(false)));
        assert_eq!(get("fort.where_attacked"), Some(Value::Bool(false)));
        assert_eq!(get("def.base_aa_towers"), None, "blank cell");
        assert_eq!(rush.settings.len(), 16);
        assert_eq!(reg.get("Turtle").unwrap().settings.len(), 11);
        assert_eq!(reg.get("Tech Rush").unwrap().settings.len(), 8);
    }

    #[test]
    fn a_file_replaces_a_builtin_in_place() {
        let mut reg = Registry::builtin();
        reg.add_file_text("rush", "def.towers=3").unwrap();
        assert_eq!(reg.all().len(), 7);
        assert_eq!(reg.all()[1].name, "rush");
        assert_eq!(reg.all()[1].settings.len(), 1);
        reg.add_file_text("Mine", "name=Mine").unwrap();
        assert_eq!(reg.all().len(), 8);
        assert!(reg.add_file_text("bad", "no equals here").is_err());
        assert_eq!(reg.all().len(), 8);
    }

    #[test]
    fn load_dir_reads_txt_in_name_order_and_skips_bad_files() {
        let dir =
            std::env::temp_dir().join(format!("btm-ai-personality-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("b.txt"), "name=Two\ndef.towers=2").unwrap();
        fs::write(dir.join("a.txt"), "name=One\ndef.towers=1").unwrap();
        fs::write(dir.join("broken.txt"), "def.towers=lots").unwrap();
        fs::write(dir.join("notes.md"), "ignored").unwrap();
        fs::write(dir.join("binary.txt"), [0xff, 0xfe, 0x00]).unwrap();
        let mut reg = Registry::builtin();
        let report = reg.load_dir(&dir).unwrap();
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(report.loaded, 2);
        assert_eq!(report.skipped.len(), 2);
        assert!(reg.get("One").is_some() && reg.get("Two").is_some());
        let tail: Vec<_> = reg
            .all()
            .iter()
            .rev()
            .take(2)
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(tail, ["Two", "One"]);
        assert!(Registry::default().load_dir(&dir).is_err());
    }
}
