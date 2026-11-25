//! Whole games in the sim: AI seats and scripted seats played together, to a
//! decision or a time cap. This is what the integration tests and the
//! arena's [`crate::arena::Match`] for the sim ([`super::SimMatch`]) run on.

use std::fmt;

use super::{Map, Script, Sim, SimConfig, SimError};
use crate::brain::{Ai, BrainError, DEFAULT_ORDER_DELAY};
use crate::personality::Resolved;
use crate::view::{OrderRecorder, PlayerId, Tick};

/// Who plays a seat.
#[derive(Clone, Debug)]
pub enum Pilot {
    /// This AI, with its resolved tuning.
    Ai(Box<Resolved>),
    /// A scripted opponent.
    Script(Script),
}

/// Why a game could not be set up or played.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameError {
    Sim(SimError),
    Brain(BrainError),
    /// The pilots do not match the configured players.
    SeatCount {
        pilots: usize,
        players: usize,
    },
}

impl fmt::Display for GameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GameError::Sim(e) => write!(f, "sim: {e}"),
            GameError::Brain(e) => write!(f, "ai: {e}"),
            GameError::SeatCount { pilots, players } => {
                write!(f, "{pilots} pilots for {players} players")
            }
        }
    }
}

impl std::error::Error for GameError {}

impl From<SimError> for GameError {
    fn from(e: SimError) -> Self {
        GameError::Sim(e)
    }
}

impl From<BrainError> for GameError {
    fn from(e: BrainError) -> Self {
        GameError::Brain(e)
    }
}

/// A game in progress.
#[derive(Debug)]
pub struct Game {
    /// The simulation.
    pub sim: Sim,
    /// The AI seats, in seat order.
    pub ais: Vec<Ai>,
}

impl Game {
    /// Sets up a game: one pilot per configured player, in seat order. Each
    /// AI seat draws its seed from the sim's stream in seat order (SPEC §7),
    /// and the sim's order delay is the fixed AI depth.
    pub fn new(map: Map, mut cfg: SimConfig, pilots: Vec<Pilot>) -> Result<Game, GameError> {
        if pilots.len() != cfg.players.len() {
            return Err(GameError::SeatCount {
                pilots: pilots.len(),
                players: cfg.players.len(),
            });
        }
        cfg.order_delay = DEFAULT_ORDER_DELAY;
        for (setup, pilot) in cfg.players.iter_mut().zip(&pilots) {
            if let Pilot::Ai(r) = pilot {
                setup.full_vision = r.knobs.cheat.full_vision;
                setup.income_factor_permille = r.knobs.cheat.income_factor.apply(1000);
            }
        }
        let mut sim = Sim::new(map, cfg)?;
        let mut ais = Vec::new();
        for (i, pilot) in pilots.into_iter().enumerate() {
            let p = PlayerId(u8::try_from(i).unwrap_or(u8::MAX));
            match pilot {
                Pilot::Ai(r) => {
                    let seed = sim.draw_seat_seed();
                    ais.push(Ai::new(p, &r, u64::from(seed))?);
                }
                Pilot::Script(s) => sim.set_script(p, s),
            }
        }
        Ok(Game { sim, ais })
    }

    /// One tick: every living AI seat thinks, its orders go into the command
    /// buffer, then the sim steps (scripts think inside the step).
    pub fn step(&mut self) -> Result<(), GameError> {
        for ai in &mut self.ais {
            if !self.sim.player_alive(ai.player()) {
                continue;
            }
            let mut rec = OrderRecorder::new();
            let v = self.sim.view(ai.player());
            ai.tick(&v, &v, &mut rec)?;
            self.sim.submit(ai.player(), rec.take());
        }
        self.sim.step();
        Ok(())
    }

    /// Steps until the game is decided or `cap` ticks have passed in all.
    /// Returns whether it was decided.
    pub fn run_to(&mut self, cap: Tick) -> Result<bool, GameError> {
        while self.sim.tick() < cap {
            if self.sim.decided() {
                // The game ends one tick after the decision (SPEC §8.1).
                self.step()?;
                return Ok(true);
            }
            self.step()?;
        }
        Ok(self.sim.decided())
    }

    /// The AI seat for `p`, if `p` is played by the AI.
    pub fn ai(&self, p: PlayerId) -> Option<&Ai> {
        self.ais.iter().find(|a| a.player() == p)
    }
}
