//! Personalities, difficulty tiers and faction defaults (SPEC.md section 6).
//!
//! A seat's tuning is assembled by [`resolve`] in the order of section 6.1,
//! later layers overwriting earlier ones:
//!
//! 1. the Appendix A defaults (the Standard profile),
//! 2. the difficulty profile ([`Difficulty`]); the personality's own
//!    difficulty if it names one, else the game's,
//! 3. the faction defaults ([`Faction`]),
//! 4. the personality's knobs, in file order,
//! 5. arena overrides, in command-line order.
//!
//! Section 6.1 of the spec is followed here (difficulty before faction); it is
//! the only order under which the section 6.3 table can say that CORE's
//! `army.wave_size` is "tier + 6".

mod builtin;
mod file;

use std::fmt;

pub use builtin::{LoadError, LoadReport, Registry, MAX_FILES};
pub use file::{
    parse_personality, ParseError, ParseErrorKind, MAX_FILE_BYTES, MAX_LINES, MAX_LINE_BYTES,
};

use crate::knobs::{KnobError, Knobs, Value};

/// The engine name a personality file may give (`engine=`) to say it is this
/// AI's. A file naming any other engine belongs to that engine.
pub const OWN_ENGINE: &str = "btm-ai";

/// A seat's side. Any side that is not CORE is treated as ARM for the role
/// table and the faction defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Faction {
    /// ARM (and every side that is not CORE).
    Arm,
    /// CORE.
    Core,
}

impl Faction {
    /// Maps a side name to a faction: `core` (any case) is CORE and anything
    /// else is ARM.
    pub fn from_name(name: &str) -> Faction {
        if name.trim().eq_ignore_ascii_case("core") {
            Faction::Core
        } else {
            Faction::Arm
        }
    }

    /// The upper-case name, `ARM` or `CORE`.
    pub fn name(self) -> &'static str {
        match self {
            Faction::Arm => "ARM",
            Faction::Core => "CORE",
        }
    }
}

/// Section 6.3 knob rows: (knob, ARM, CORE). `None` means unchanged.
const FACTION_TABLE: &[(&str, Option<&str>, Option<&str>)] = &[
    ("mix.lab_raider", Some("0"), Some("0")),
    ("mix.lab_rocket", Some("0"), Some("4")),
    ("mix.lab_artillery", None, Some("2")),
    ("mix.vp_light_tank", None, Some("3")),
    ("mix.vp_missile", None, Some("1")),
    ("mix.vp_medium_tank", None, Some("2")),
    ("mix.counter_bonus", None, Some("2")),
    ("mix.counter_trigger", None, Some("0.3606")),
];

/// CORE's `army.wave_size` is the tier's plus this much.
const CORE_WAVE_BONUS: i32 = 6;

/// Applies the faction defaults of section 6.3 on top of the difficulty.
fn apply_faction(knobs: &mut Knobs, faction: Faction) -> Result<(), KnobError> {
    for (name, arm, core) in FACTION_TABLE {
        let text = match faction {
            Faction::Arm => arm,
            Faction::Core => core,
        };
        if let Some(text) = text {
            knobs.set_text(name, text)?;
        }
    }
    if faction == Faction::Core {
        knobs.army.wave_size = knobs.army.wave_size.saturating_add(CORE_WAVE_BONUS);
    }
    Ok(())
}

/// A difficulty tier (section 6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Difficulty {
    /// Constructs the seat (it still draws its random seed) but never issues
    /// anything.
    Idle,
    /// Fewer buildings, bigger waves, slower cadence.
    Easy,
    /// The Standard profile, equal to the Appendix A defaults.
    Standard,
    /// More buildings, smaller waves, faster cadence.
    Hard,
    /// Hard's successor: also cheats (full vision and a 1.25 income factor).
    Brutal,
}

/// Section 6.2: (knob, Easy, Standard, Hard, Brutal).
const TIER_TABLE: &[(&str, [&str; 4])] = &[
    ("eco.solar", ["6", "10", "12", "10"]),
    ("eco.mex", ["5", "8", "10", "8"]),
    ("build.constructors", ["1", "1", "2", "3"]),
    ("eco.extra_labs", ["0", "1", "2", "1"]),
    ("build.vehicle_plants", ["0", "1", "1", "1"]),
    ("def.towers", ["1", "2", "3", "3"]),
    ("def.base_aa_towers", ["0", "1", "2", "2"]),
    ("def.reactive_aa_towers", ["1", "3", "5", "5"]),
    ("def.mobile_aa", ["0", "2", "3", "3"]),
    ("army.wave_size", ["14", "8", "7", "6"]),
    ("army.spent_size", ["5", "3", "2", "3"]),
    ("cadence.plan_ticks", ["60", "30", "20", "20"]),
    ("cadence.tactics_ticks", ["30", "15", "10", "10"]),
    ("cadence.scout_ticks", ["120", "60", "45", "60"]),
    ("def.face_losses", ["off", "on", "on", "on"]),
    ("def.payback_s", ["0", "90", "60", "45"]),
    ("def.payback_per_mex_s", ["45", "45", "30", "20"]),
    ("build.site_guard_size", ["0", "0", "3", "3"]),
    ("build.site_guard_threat", ["0", "0", "1000", "1000"]),
    ("navy.fleet", ["3", "6", "9", "12"]),
    ("navy.subs", ["0", "2", "3", "4"]),
    ("navy.subs_after_destroyers", ["3", "3", "2", "2"]),
    ("reclaim.own_reclaimers", ["1", "1", "2", "2"]),
    ("reclaim.own_delay_s", ["30", "10", "0", "0"]),
    ("reclaim.own_only_when_short", ["on", "off", "off", "off"]),
    ("cheat.full_vision", ["off", "off", "off", "on"]),
    ("cheat.income_factor", ["1", "1", "1", "1.25"]),
];

/// What a lobby needs to know about a difficulty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DifficultyProfile {
    /// The tier.
    pub difficulty: Difficulty,
    /// The display name.
    pub name: &'static str,
    /// True if the tier cheats (full vision, income above 1), so the lobby
    /// can label it.
    pub cheats: bool,
    /// The micro limiter's cap in orders per minute (section 2.3), or `None`
    /// for no cap. Idle issues nothing, so it has none.
    pub micro_limit: Option<u32>,
}

impl Difficulty {
    /// Every tier, weakest first.
    pub const ALL: [Difficulty; 5] = [
        Difficulty::Idle,
        Difficulty::Easy,
        Difficulty::Standard,
        Difficulty::Hard,
        Difficulty::Brutal,
    ];

    /// Parses the `difficulty=` values of section 6.5: `idle` / `none` /
    /// `off`, `easy`, `standard` / `medium` / `normal`, `hard`, `brutal`
    /// (any case).
    pub fn from_name(name: &str) -> Option<Difficulty> {
        let n = name.trim().to_ascii_lowercase();
        match n.as_str() {
            "idle" | "none" | "off" => Some(Difficulty::Idle),
            "easy" => Some(Difficulty::Easy),
            "standard" | "medium" | "normal" => Some(Difficulty::Standard),
            "hard" => Some(Difficulty::Hard),
            "brutal" => Some(Difficulty::Brutal),
            _ => None,
        }
    }

    /// The tier's table column. Idle uses the Standard profile.
    fn column(self) -> usize {
        match self {
            Difficulty::Easy => 0,
            Difficulty::Idle | Difficulty::Standard => 1,
            Difficulty::Hard => 2,
            Difficulty::Brutal => 3,
        }
    }

    /// The tier's facts for a lobby.
    pub fn profile(self) -> DifficultyProfile {
        DifficultyProfile {
            difficulty: self,
            name: match self {
                Difficulty::Idle => "Idle",
                Difficulty::Easy => "Easy",
                Difficulty::Standard => "Standard",
                Difficulty::Hard => "Hard",
                Difficulty::Brutal => "Brutal",
            },
            cheats: self == Difficulty::Brutal,
            micro_limit: match self {
                Difficulty::Easy => Some(60),
                Difficulty::Standard => Some(120),
                Difficulty::Idle | Difficulty::Hard | Difficulty::Brutal => None,
            },
        }
    }

    /// True if the tier cheats.
    pub fn cheats(self) -> bool {
        self.profile().cheats
    }

    /// Applies this tier's section 6.2 knobs to `knobs`. Idle also sets
    /// `misc.idle`.
    pub fn apply(self, knobs: &mut Knobs) -> Result<(), KnobError> {
        let col = self.column();
        for (name, values) in TIER_TABLE {
            if let Some(text) = values.get(col) {
                knobs.set_text(name, text)?;
            }
        }
        if self == Difficulty::Idle {
            knobs.set("misc.idle", Value::Bool(true))?;
        }
        Ok(())
    }
}

/// One knob a personality sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Setting {
    /// The dotted knob name.
    pub knob: &'static str,
    /// The value, already typed.
    pub value: Value,
}

/// A personality: a name, an optional difficulty and an ordered list of knob
/// settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Personality {
    /// Its name (case-insensitive when looked up).
    pub name: String,
    /// A one-line description, possibly empty.
    pub description: String,
    /// The difficulty it names, if any; else the game's applies.
    pub difficulty: Option<Difficulty>,
    /// The engine it names, if any. `None` means this AI.
    pub engine: Option<String>,
    /// The knobs it sets, in file order.
    pub settings: Vec<Setting>,
}

impl Personality {
    /// An empty personality (all defaults) with the given name.
    pub fn new(name: &str) -> Personality {
        Personality {
            name: name.to_string(),
            description: String::new(),
            difficulty: None,
            engine: None,
            settings: Vec::new(),
        }
    }

    /// True unless the personality names another engine.
    pub fn is_own_engine(&self) -> bool {
        match self.engine.as_deref() {
            None => true,
            Some(e) => e.is_empty() || e.eq_ignore_ascii_case(OWN_ENGINE),
        }
    }

    /// True if the personality itself turns on a cheat knob, or names a
    /// cheating difficulty. (A game difficulty of Brutal is seen only in
    /// [`Resolved::cheats`].)
    pub fn cheats(&self) -> bool {
        self.difficulty.is_some_and(Difficulty::cheats)
            || self.settings.iter().any(|s| match (s.knob, s.value) {
                ("cheat.full_vision", Value::Bool(b)) => b,
                ("cheat.income_factor", Value::Num(n)) => n > crate::knobs::Fixed::ONE,
                _ => false,
            })
    }
}

/// An error from [`resolve`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// The personality belongs to another engine and cannot tune this AI.
    OtherEngine(String),
    /// An arena override named an unknown knob or had a bad value. Unknown
    /// knobs are fatal here.
    Override(KnobError),
    /// A built-in table failed to apply (a bug; covered by tests).
    Table(KnobError),
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveError::OtherEngine(e) => {
                write!(f, "personality is for engine `{e}`, not this AI")
            }
            ResolveError::Override(e) => write!(f, "override: {e}"),
            ResolveError::Table(e) => write!(f, "internal table error: {e}"),
        }
    }
}

impl std::error::Error for ResolveError {}

/// A seat's finished tuning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The assembled knobs.
    pub knobs: Knobs,
    /// The difficulty that was applied.
    pub difficulty: Difficulty,
    /// The faction defaults that were applied.
    pub faction: Faction,
    /// True if the final knobs cheat (full vision or income factor above 1),
    /// whether through the tier, the personality or an override.
    pub cheats: bool,
    /// The micro limiter's orders-per-minute cap, or `None` for no cap.
    pub micro_limit: Option<u32>,
}

/// Assembles a seat's knobs (section 6.1).
///
/// `game_difficulty` is used unless the personality names its own.
/// `overrides` are arena `knob=value` pairs in command-line order; any knob,
/// including the ones hidden from personality files, may be overridden.
pub fn resolve(
    personality: &Personality,
    game_difficulty: Difficulty,
    faction: Faction,
    overrides: &[(&str, &str)],
) -> Result<Resolved, ResolveError> {
    if !personality.is_own_engine() {
        return Err(ResolveError::OtherEngine(
            personality.engine.clone().unwrap_or_default(),
        ));
    }
    let difficulty = personality.difficulty.unwrap_or(game_difficulty);
    let mut knobs = Knobs::default();
    difficulty.apply(&mut knobs).map_err(ResolveError::Table)?;
    apply_faction(&mut knobs, faction).map_err(ResolveError::Table)?;
    for s in &personality.settings {
        knobs.set(s.knob, s.value).map_err(ResolveError::Table)?;
    }
    for (name, text) in overrides {
        knobs.set_text(name, text).map_err(ResolveError::Override)?;
    }
    Ok(Resolved {
        cheats: knobs.cheats(),
        micro_limit: difficulty.profile().micro_limit,
        knobs,
        difficulty,
        faction,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::knobs::{knob_info, Fixed};

    fn builtin(name: &str) -> Personality {
        Registry::builtin().get(name).cloned().unwrap()
    }

    fn resolved(name: &str, d: Difficulty, f: Faction) -> Resolved {
        resolve(&builtin(name), d, f, &[]).unwrap()
    }

    #[test]
    fn tables_name_real_knobs_of_the_right_shape() {
        for (name, values) in TIER_TABLE {
            assert!(knob_info(name).is_some(), "{name}");
            for d in Difficulty::ALL {
                let mut k = Knobs::default();
                d.apply(&mut k).unwrap();
            }
            assert_eq!(values.len(), 4);
        }
        for (name, _, _) in FACTION_TABLE {
            assert!(knob_info(name).is_some(), "{name}");
        }
    }

    #[test]
    fn standard_tier_equals_the_appendix_a_defaults() {
        let mut k = Knobs::default();
        Difficulty::Standard.apply(&mut k).unwrap();
        assert_eq!(k, Knobs::default());
    }

    #[test]
    fn tier_values_from_the_table() {
        let r = |d| resolved("Balanced", d, Faction::Arm).knobs;
        let easy = r(Difficulty::Easy);
        assert_eq!(easy.eco.solar, 6);
        assert_eq!(easy.army.wave_size, 14);
        assert!(!easy.def.face_losses);
        assert_eq!(easy.def.payback_s, 0);
        assert!(easy.reclaim.own_only_when_short);
        let hard = r(Difficulty::Hard);
        assert_eq!(hard.build.constructors, 2);
        assert_eq!(hard.cadence.plan_ticks, 20);
        assert_eq!(hard.build.site_guard_threat, Fixed::from_int(1000));
        let brutal = r(Difficulty::Brutal);
        assert_eq!(brutal.navy.fleet, 12);
        assert_eq!(brutal.navy.subs, 4);
        assert_eq!(brutal.cadence.scout_ticks, 60);
        assert_eq!(brutal.def.payback_per_mex_s, 20);
    }

    #[test]
    fn brutal_cheats_and_nothing_else_does() {
        for d in Difficulty::ALL {
            let r = resolved("Balanced", d, Faction::Arm);
            assert_eq!(r.cheats, d == Difficulty::Brutal, "{d:?}");
            assert_eq!(d.profile().cheats, d == Difficulty::Brutal);
        }
        let b = resolved("Balanced", Difficulty::Brutal, Faction::Arm);
        assert!(b.knobs.cheat.full_vision);
        assert_eq!(b.knobs.cheat.income_factor.raw(), 12_500);
        assert_eq!(b.knobs.cheat.income_factor.apply(100), 125);
        assert!(!builtin("Rush").cheats());
    }

    #[test]
    fn micro_limits() {
        assert_eq!(Difficulty::Easy.profile().micro_limit, Some(60));
        assert_eq!(Difficulty::Standard.profile().micro_limit, Some(120));
        assert_eq!(Difficulty::Hard.profile().micro_limit, None);
        assert_eq!(Difficulty::Brutal.profile().micro_limit, None);
    }

    #[test]
    fn idle_sets_the_idle_knob() {
        let r = resolved("Balanced", Difficulty::Idle, Faction::Arm);
        assert!(r.knobs.misc.idle);
        assert!(
            !resolved("Balanced", Difficulty::Standard, Faction::Arm)
                .knobs
                .misc
                .idle
        );
    }

    #[test]
    fn faction_defaults() {
        let arm = resolved("Balanced", Difficulty::Standard, Faction::Arm).knobs;
        assert_eq!(arm.mix.lab_raider, 0);
        assert_eq!(arm.mix.lab_rocket, 0);
        assert_eq!(arm.mix.lab_artillery, 0);
        assert_eq!(arm.mix.vp_light_tank, 1);
        assert_eq!(arm.mix.counter_trigger, Fixed::from_raw(3_000));
        assert_eq!(arm.army.wave_size, 8);
        let core = resolved("Balanced", Difficulty::Standard, Faction::Core).knobs;
        assert_eq!(core.mix.lab_raider, 0);
        assert_eq!(core.mix.lab_rocket, 4);
        assert_eq!(core.mix.lab_artillery, 2);
        assert_eq!(core.mix.vp_light_tank, 3);
        assert_eq!(core.mix.vp_missile, 1);
        assert_eq!(core.mix.vp_medium_tank, 2);
        assert_eq!(core.mix.counter_bonus, 2);
        assert_eq!(core.mix.counter_trigger.to_string(), "0.3606");
        assert_eq!(Faction::from_name("Core"), Faction::Core);
        assert_eq!(Faction::from_name("lizard"), Faction::Arm);
    }

    #[test]
    fn core_wave_size_is_tier_plus_six() {
        for (d, want) in [
            (Difficulty::Easy, 20),
            (Difficulty::Standard, 14),
            (Difficulty::Hard, 13),
            (Difficulty::Brutal, 12),
        ] {
            let k = resolved("Balanced", d, Faction::Core).knobs;
            assert_eq!(k.army.wave_size, want, "{d:?}");
        }
    }

    #[test]
    fn resolve_order_defaults_tier_faction_personality_override() {
        // Defaults: lab_raider 2, wave_size 8, towers 2.
        let plain = Personality::new("p");
        let k = resolve(&plain, Difficulty::Standard, Faction::Arm, &[])
            .unwrap()
            .knobs;
        assert_eq!(
            (k.mix.lab_raider, k.army.wave_size, k.def.towers),
            (0, 8, 2)
        );

        // Tier beats the defaults.
        let k = resolve(&plain, Difficulty::Hard, Faction::Arm, &[])
            .unwrap()
            .knobs;
        assert_eq!((k.army.wave_size, k.def.towers), (7, 3));

        // Faction beats the tier: CORE adds to the tier's wave size.
        let k = resolve(&plain, Difficulty::Hard, Faction::Core, &[])
            .unwrap()
            .knobs;
        assert_eq!(k.army.wave_size, 13);

        // Personality beats faction and tier: Rush on CORE builds raiders and
        // a wave of exactly 5, whatever the tier.
        let k = resolved("Rush", Difficulty::Hard, Faction::Core).knobs;
        assert_eq!(k.mix.lab_raider, 4);
        assert_eq!(k.mix.lab_rocket, 1);
        assert_eq!(k.mix.lab_artillery, 0);
        assert_eq!(k.army.wave_size, 5);
        assert_eq!(k.def.towers, 0);
        // ...while knobs the personality leaves blank keep the lower layers.
        assert_eq!(k.mix.vp_light_tank, 3);
        assert_eq!(k.def.reactive_aa_towers, 5);

        // Overrides beat everything and apply in command-line order.
        let k = resolve(
            &builtin("Rush"),
            Difficulty::Hard,
            Faction::Core,
            &[
                ("army.wave_size", "9"),
                ("def.towers", "4"),
                ("army.wave_size", "11"),
            ],
        )
        .unwrap()
        .knobs;
        assert_eq!((k.army.wave_size, k.def.towers), (11, 4));
    }

    #[test]
    fn personality_difficulty_wins_over_the_games() {
        let r = resolved("Easy", Difficulty::Brutal, Faction::Arm);
        assert_eq!(r.difficulty, Difficulty::Easy);
        assert!(!r.cheats);
        assert_eq!(r.knobs.eco.solar, 6);
        let r = resolved("Hard", Difficulty::Easy, Faction::Arm);
        assert_eq!(r.difficulty, Difficulty::Hard);
        // A personality with no difficulty takes the game's.
        let r = resolved("Turtle", Difficulty::Hard, Faction::Arm);
        assert_eq!(r.difficulty, Difficulty::Hard);
        assert_eq!(r.knobs.def.towers, 6);
        assert_eq!(r.knobs.def.base_aa_towers, 2);
        assert_eq!(r.knobs.def.reactive_aa_towers, 5);
    }

    #[test]
    fn a_file_can_turn_cheats_on_or_off() {
        let p = parse_personality("difficulty=brutal\ncheat.full_vision=off", "p").unwrap();
        assert!(p.cheats());
        let r = resolve(&p, Difficulty::Easy, Faction::Arm, &[]).unwrap();
        assert!(r.cheats, "income factor is still 1.25");
        let p = parse_personality("cheat.income_factor=2", "p").unwrap();
        assert!(p.cheats());
    }

    #[test]
    fn bad_overrides_and_foreign_engines_are_errors() {
        let p = Personality::new("p");
        let e = resolve(
            &p,
            Difficulty::Standard,
            Faction::Arm,
            &[("army.nope", "1")],
        )
        .unwrap_err();
        assert_eq!(
            e,
            ResolveError::Override(KnobError::Unknown("army.nope".into()))
        );
        let e = resolve(
            &p,
            Difficulty::Standard,
            Faction::Arm,
            &[("army.rally", "x")],
        )
        .unwrap_err();
        assert!(matches!(
            e,
            ResolveError::Override(KnobError::BadValue { .. })
        ));
        // Hidden knobs can be overridden by the arena.
        let k = resolve(
            &p,
            Difficulty::Standard,
            Faction::Arm,
            &[("scout.enabled", "0")],
        )
        .unwrap()
        .knobs;
        assert_eq!(k.scout.enabled, 0);
        let other = parse_personality("engine=classic", "c").unwrap();
        assert!(matches!(
            resolve(&other, Difficulty::Standard, Faction::Arm, &[]),
            Err(ResolveError::OtherEngine(_))
        ));
    }
}
