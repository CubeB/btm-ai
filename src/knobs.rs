//! The tuning knobs of SPEC.md Appendix A.
//!
//! Every knob is a typed field of [`Knobs`], grouped into one sub-struct per
//! name prefix (`knobs.army.wave_size` is the knob `army.wave_size`). The
//! three knob types are:
//!
//! - `bool`;
//! - `i32` (the spec's `int`);
//! - [`Fixed`], a decimal fixed-point number with four fractional digits (the
//!   spec's `number`).
//!
//! No knob is a float, so every value that can influence an order is exact
//! and identical on every peer (SPEC.md section 7).
//!
//! Knobs can also be reached by their dotted name, which is how personality
//! files and arena overrides address them: [`Knobs::get`], [`Knobs::set`] and
//! [`Knobs::set_text`]. [`KNOB_TABLE`] lists every knob.

use std::fmt;

/// Number of raw units in one whole [`Fixed`] unit.
const FIXED_SCALE: i64 = 10_000;

/// A decimal fixed-point number: a signed count of ten-thousandths.
///
/// This is the type of the spec's `number` knobs. `1.25` is `Fixed(12_500)`
/// and `0.3606` is `Fixed(3_606)`. Distances are in world units, so a radius
/// of 96 is `Fixed::from_int(96)`. Equality and ordering are those of the
/// underlying integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Fixed(i64);

impl Fixed {
    /// Zero.
    pub const ZERO: Fixed = Fixed(0);
    /// One.
    pub const ONE: Fixed = Fixed(FIXED_SCALE);

    /// Builds a value from a raw count of ten-thousandths.
    pub const fn from_raw(raw: i64) -> Fixed {
        Fixed(raw)
    }

    /// The raw count of ten-thousandths.
    pub const fn raw(self) -> i64 {
        self.0
    }

    /// Builds a whole number.
    pub const fn from_int(n: i32) -> Fixed {
        Fixed(n as i64 * FIXED_SCALE)
    }

    /// The value rounded towards negative infinity to a whole number.
    pub const fn floor(self) -> i64 {
        self.0.div_euclid(FIXED_SCALE)
    }

    /// Multiplies a whole amount by this factor, rounding towards negative
    /// infinity. Used for the income factor: `Fixed(12_500).apply(8) == 10`.
    /// Saturates at the `i64` limits.
    pub fn apply(self, amount: i64) -> i64 {
        let wide = i128::from(amount) * i128::from(self.0);
        let scaled = wide.div_euclid(i128::from(FIXED_SCALE));
        i64::try_from(scaled).unwrap_or(if scaled < 0 { i64::MIN } else { i64::MAX })
    }

    /// The nearest `f64`. For reports only; never use it to decide an order.
    pub fn to_f64(self) -> f64 {
        self.0 as f64 / FIXED_SCALE as f64
    }

    /// Parses a plain decimal such as `12`, `-3`, `0.5` or `1.2500`.
    ///
    /// At most four significant fractional digits are accepted (further
    /// digits must be zero), and exponents are not.
    pub fn parse(text: &str) -> Result<Fixed, FixedParseError> {
        let s = text.trim();
        let (neg, digits) = match s.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, s.strip_prefix('+').unwrap_or(s)),
        };
        let (whole, frac) = match digits.split_once('.') {
            Some((w, f)) => (w, f),
            None => (digits, ""),
        };
        if whole.is_empty() && frac.is_empty() {
            return Err(FixedParseError::Empty);
        }
        if !whole.bytes().all(|b| b.is_ascii_digit()) || !frac.bytes().all(|b| b.is_ascii_digit()) {
            return Err(FixedParseError::NotANumber);
        }
        let mut raw: i64 = 0;
        for b in whole.bytes() {
            raw = raw
                .checked_mul(10)
                .and_then(|r| r.checked_add(i64::from(b - b'0')))
                .ok_or(FixedParseError::OutOfRange)?;
        }
        raw = raw
            .checked_mul(FIXED_SCALE)
            .ok_or(FixedParseError::OutOfRange)?;
        let mut place = FIXED_SCALE / 10;
        for b in frac.bytes() {
            let d = i64::from(b - b'0');
            if place == 0 {
                if d != 0 {
                    return Err(FixedParseError::TooPrecise);
                }
                continue;
            }
            raw = raw
                .checked_add(d * place)
                .ok_or(FixedParseError::OutOfRange)?;
            place /= 10;
        }
        Ok(Fixed(if neg { -raw } else { raw }))
    }
}

impl fmt::Display for Fixed {
    /// Prints the shortest exact decimal: `96`, `1.25`, `0.3606`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let abs = self.0.unsigned_abs();
        let scale = FIXED_SCALE.unsigned_abs();
        let sign = if self.0 < 0 { "-" } else { "" };
        let whole = abs / scale;
        let frac = abs % scale;
        if frac == 0 {
            write!(f, "{sign}{whole}")
        } else {
            let text = format!("{frac:04}");
            write!(f, "{sign}{whole}.{}", text.trim_end_matches('0'))
        }
    }
}

/// Why [`Fixed::parse`] rejected a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixedParseError {
    /// There were no digits.
    Empty,
    /// The text is not a plain decimal number.
    NotANumber,
    /// The value does not fit.
    OutOfRange,
    /// More than four fractional digits.
    TooPrecise,
}

impl fmt::Display for FixedParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            FixedParseError::Empty => "empty number",
            FixedParseError::NotANumber => "not a decimal number",
            FixedParseError::OutOfRange => "number out of range",
            FixedParseError::TooPrecise => "more than four fractional digits",
        })
    }
}

impl std::error::Error for FixedParseError {}

/// The type of a knob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnobKind {
    /// `bool`.
    Bool,
    /// `i32`.
    Int,
    /// [`Fixed`].
    Num,
}

impl fmt::Display for KnobKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            KnobKind::Bool => "bool",
            KnobKind::Int => "int",
            KnobKind::Num => "number",
        })
    }
}

/// A knob value of any type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Value {
    /// A boolean knob.
    Bool(bool),
    /// An integer knob.
    Int(i32),
    /// A fixed-point knob.
    Num(Fixed),
}

impl Value {
    /// The type of this value.
    pub fn kind(self) -> KnobKind {
        match self {
            Value::Bool(_) => KnobKind::Bool,
            Value::Int(_) => KnobKind::Int,
            Value::Num(_) => KnobKind::Num,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Num(n) => write!(f, "{n}"),
        }
    }
}

/// A Rust type that can be a knob field.
pub trait KnobType: Copy {
    /// The knob kind this type stands for.
    const KIND: KnobKind;
    /// Wraps the value.
    fn to_value(self) -> Value;
    /// Unwraps a value of this type, or `None` for another type.
    fn from_value(value: Value) -> Option<Self>;
}

impl KnobType for bool {
    const KIND: KnobKind = KnobKind::Bool;
    fn to_value(self) -> Value {
        Value::Bool(self)
    }
    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::Bool(b) => Some(b),
            _ => None,
        }
    }
}

impl KnobType for i32 {
    const KIND: KnobKind = KnobKind::Int;
    fn to_value(self) -> Value {
        Value::Int(self)
    }
    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::Int(i) => Some(i),
            _ => None,
        }
    }
}

impl KnobType for Fixed {
    const KIND: KnobKind = KnobKind::Num;
    fn to_value(self) -> Value {
        Value::Num(self)
    }
    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::Num(n) => Some(n),
            _ => None,
        }
    }
}

/// One row of [`KNOB_TABLE`]: the static description of a knob.
pub struct KnobInfo {
    /// The dotted name, such as `army.wave_size`.
    pub name: &'static str,
    /// The prefix (the sub-struct), such as `army`.
    pub group: &'static str,
    /// The knob's type.
    pub kind: KnobKind,
    /// False for the knobs SPEC.md marks "not exposed to personality files".
    /// Arena overrides may still set them.
    pub exposed: bool,
    pub(crate) get: fn(&Knobs) -> Value,
    pub(crate) set: fn(&mut Knobs, Value) -> bool,
}

impl KnobInfo {
    /// Parses the text of a knob value as the personality-file format of
    /// SPEC.md section 6.5 defines it: `1`, `true`, `on` and `yes` (any case)
    /// are true and any other text is false; integers must be plain integers;
    /// numbers are plain decimals.
    pub fn parse_value(&self, text: &str) -> Result<Value, KnobError> {
        let text = text.trim();
        let bad = || KnobError::BadValue {
            name: self.name.to_string(),
            text: text.to_string(),
            expected: self.kind,
        };
        match self.kind {
            KnobKind::Bool => Ok(Value::Bool(
                ["1", "true", "on", "yes"]
                    .iter()
                    .any(|t| text.eq_ignore_ascii_case(t)),
            )),
            KnobKind::Int => text.parse::<i32>().map(Value::Int).map_err(|_| bad()),
            KnobKind::Num => Fixed::parse(text).map(Value::Num).map_err(|_| bad()),
        }
    }
}

impl fmt::Debug for KnobInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KnobInfo")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("exposed", &self.exposed)
            .finish()
    }
}

/// An error from the by-name knob accessors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnobError {
    /// No knob has this name.
    Unknown(String),
    /// The value has the wrong type for the knob.
    TypeMismatch {
        /// The knob.
        name: String,
        /// The knob's type.
        expected: KnobKind,
        /// The type supplied.
        got: KnobKind,
    },
    /// The text does not parse as the knob's type.
    BadValue {
        /// The knob.
        name: String,
        /// The offending text.
        text: String,
        /// The knob's type.
        expected: KnobKind,
    },
}

impl fmt::Display for KnobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KnobError::Unknown(n) => write!(f, "unknown knob `{n}`"),
            KnobError::TypeMismatch {
                name,
                expected,
                got,
            } => write!(f, "knob `{name}` is a {expected}, not a {got}"),
            KnobError::BadValue {
                name,
                text,
                expected,
            } => write!(f, "`{text}` is not a valid {expected} for knob `{name}`"),
        }
    }
}

impl std::error::Error for KnobError {}

/// Finds a knob by its dotted name (exact, case-sensitive).
pub fn knob_info(name: &str) -> Option<&'static KnobInfo> {
    KNOB_TABLE.iter().find(|k| k.name == name)
}

impl Knobs {
    /// Reads a knob by its dotted name.
    pub fn get(&self, name: &str) -> Option<Value> {
        knob_info(name).map(|k| (k.get)(self))
    }

    /// Writes a knob by its dotted name. The value's type must match the
    /// knob's.
    pub fn set(&mut self, name: &str, value: Value) -> Result<(), KnobError> {
        let info = knob_info(name).ok_or_else(|| KnobError::Unknown(name.to_string()))?;
        if (info.set)(self, value) {
            Ok(())
        } else {
            Err(KnobError::TypeMismatch {
                name: name.to_string(),
                expected: info.kind,
                got: value.kind(),
            })
        }
    }

    /// Parses `text` as the knob's type (see [`KnobInfo::parse_value`]) and
    /// writes it.
    pub fn set_text(&mut self, name: &str, text: &str) -> Result<(), KnobError> {
        let info = knob_info(name).ok_or_else(|| KnobError::Unknown(name.to_string()))?;
        let value = info.parse_value(text)?;
        self.set(name, value)
    }

    /// Every knob with its current value, in catalogue order.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, Value)> + '_ {
        KNOB_TABLE.iter().map(|k| (k.name, (k.get)(self)))
    }

    /// True if this seat cheats: it sees through the fog or has an income
    /// factor above 1. A lobby uses this to label the seat.
    pub fn cheats(&self) -> bool {
        self.cheat.full_vision || self.cheat.income_factor > Fixed::ONE
    }
}

/// Declares the knob structs, their defaults, the `Knobs` aggregate and
/// `KNOB_TABLE`, so that the name table can never disagree with the fields.
///
/// Each group is `field: StructName { knob: type = default, exposed; ... }`.
macro_rules! define_knobs {
    ($(
        $(#[$gmeta:meta])*
        $group:ident : $gty:ident {
            $( $(#[$meta:meta])* $knob:ident : $ty:ty = $def:expr, $exposed:expr; )*
        }
    )*) => {
        $(
            $(#[$gmeta])*
            #[derive(Debug, Clone, PartialEq, Eq)]
            pub struct $gty {
                $( $(#[$meta])* pub $knob: $ty, )*
            }

            impl Default for $gty {
                fn default() -> Self {
                    Self { $( $knob: $def, )* }
                }
            }
        )*

        /// Every knob of SPEC.md Appendix A, grouped by name prefix. The
        /// default is the Standard profile before faction defaults.
        #[derive(Debug, Clone, PartialEq, Eq, Default)]
        pub struct Knobs {
            $(
                /// See the group struct.
                pub $group: $gty,
            )*
        }

        /// Every knob, in catalogue order.
        pub static KNOB_TABLE: &[KnobInfo] = &[
            $( $(
                KnobInfo {
                    name: concat!(stringify!($group), ".", stringify!($knob)),
                    group: stringify!($group),
                    kind: <$ty as KnobType>::KIND,
                    exposed: $exposed,
                    get: |k: &Knobs| KnobType::to_value(k.$group.$knob),
                    set: |k: &mut Knobs, v: Value| match <$ty as KnobType>::from_value(v) {
                        Some(x) => {
                            k.$group.$knob = x;
                            true
                        }
                        None => false,
                    },
                },
            )* )*
        ];
    };
}

mod catalogue;

pub use catalogue::*;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The number of rows in SPEC.md Appendix A.
    const APPENDIX_A_COUNT: usize = 336;

    #[test]
    fn count_matches_appendix_a() {
        assert_eq!(KNOB_TABLE.len(), APPENDIX_A_COUNT);
        assert_eq!(Knobs::default().iter().count(), APPENDIX_A_COUNT);
    }

    #[test]
    fn names_are_unique_and_dotted() {
        let mut seen = BTreeSet::new();
        for k in KNOB_TABLE {
            assert!(seen.insert(k.name), "duplicate knob {}", k.name);
            let (group, field) = k.name.split_once('.').unwrap_or(("", ""));
            assert_eq!(group, k.group);
            assert!(!field.is_empty() && !field.contains('.'), "{}", k.name);
        }
    }

    #[test]
    fn hidden_knobs_are_the_four_the_spec_marks() {
        let hidden: Vec<_> = KNOB_TABLE
            .iter()
            .filter(|k| !k.exposed)
            .map(|k| k.name)
            .collect();
        assert_eq!(
            hidden,
            [
                "cadence.threat_grid_ticks",
                "air.transports",
                "scout.enabled",
                "ferry.pickup_timeout_s"
            ]
        );
    }

    #[test]
    fn defaults_match_spec_samples() {
        let k = Knobs::default();
        assert_eq!(k.cadence.plan_ticks, 30);
        assert!(!k.cheat.full_vision);
        assert_eq!(k.cheat.income_factor, Fixed::ONE);
        assert_eq!(k.eco.upgrade_min_metal_frac.raw(), 5_000);
        assert_eq!(k.eco.capacity_ratio.to_string(), "1.25");
        assert_eq!(k.build.claim_radius, Fixed::from_int(96));
        assert_eq!(k.mix.counter_trigger.to_string(), "0.3");
        assert_eq!(k.army.wave_size, 8);
        assert_eq!(k.army.odds_open.to_string(), "1.1");
        assert_eq!(k.army.hunt_max_escort, 600);
        assert_eq!(k.navy.yard_spacing, Fixed::from_int(448));
        assert!(!k.misc.idle);
        assert!(!k.cheats());
    }

    #[test]
    fn get_set_round_trip_every_knob() {
        let defaults = Knobs::default();
        for (name, value) in defaults.iter() {
            let mut k = Knobs::default();
            k.set(name, value).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(k, defaults, "{name}");
            let mut k2 = Knobs::default();
            k2.set_text(name, &value.to_string())
                .unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(k2.get(name), Some(value), "{name}");
        }
    }

    #[test]
    fn set_by_name_changes_the_field() {
        let mut k = Knobs::default();
        k.set("army.wave_size", Value::Int(4)).unwrap();
        k.set_text("def.towers", "0").unwrap();
        k.set_text("fort.every_tower", "On").unwrap();
        k.set_text("cheat.income_factor", "1.25").unwrap();
        assert_eq!(k.army.wave_size, 4);
        assert_eq!(k.def.towers, 0);
        assert!(k.fort.every_tower);
        assert_eq!(k.cheat.income_factor.raw(), 12_500);
        assert!(k.cheats());
        assert_eq!(k.get("army.wave_size"), Some(Value::Int(4)));
        assert_eq!(k.get("nope.nothing"), None);
    }

    #[test]
    fn errors_are_typed() {
        let mut k = Knobs::default();
        assert_eq!(
            k.set("army.wave_sizes", Value::Int(1)),
            Err(KnobError::Unknown("army.wave_sizes".into()))
        );
        assert!(matches!(
            k.set("army.wave_size", Value::Bool(true)),
            Err(KnobError::TypeMismatch {
                expected: KnobKind::Int,
                got: KnobKind::Bool,
                ..
            })
        ));
        assert!(matches!(
            k.set_text("army.wave_size", "1.5"),
            Err(KnobError::BadValue { .. })
        ));
        assert!(matches!(
            k.set_text("army.rally", "far"),
            Err(KnobError::BadValue { .. })
        ));
        // Any text is a valid boolean; anything but the true words is false.
        k.set_text("army.waves", "nonsense").unwrap();
        assert!(!k.army.waves);
    }

    #[test]
    fn fixed_parse_and_display() {
        assert_eq!(Fixed::parse("0.3606"), Ok(Fixed::from_raw(3_606)));
        assert_eq!(Fixed::parse("-1.5"), Ok(Fixed::from_raw(-15_000)));
        assert_eq!(Fixed::parse("2.50000"), Ok(Fixed::from_raw(25_000)));
        assert_eq!(Fixed::parse("2.00001"), Err(FixedParseError::TooPrecise));
        assert_eq!(Fixed::parse(""), Err(FixedParseError::Empty));
        assert_eq!(Fixed::parse("1e3"), Err(FixedParseError::NotANumber));
        assert_eq!(
            Fixed::parse("99999999999999999999"),
            Err(FixedParseError::OutOfRange)
        );
        assert_eq!(Fixed::from_raw(-5_000).to_string(), "-0.5");
        assert_eq!(Fixed::from_int(96).to_string(), "96");
        assert_eq!(Fixed::from_raw(3_606).to_string(), "0.3606");
    }

    #[test]
    fn fixed_apply_floors() {
        let f = Fixed::from_raw(12_500);
        assert_eq!(f.apply(8), 10);
        assert_eq!(f.apply(7), 8);
        assert_eq!(f.apply(-7), -9);
        assert_eq!(Fixed::ONE.apply(i64::MAX), i64::MAX);
        assert_eq!(Fixed::from_int(2).apply(i64::MAX), i64::MAX);
        assert_eq!(Fixed::from_raw(15_000).floor(), 1);
        assert_eq!(Fixed::from_raw(-15_000).floor(), -2);
    }
}
