# btm-ai

A skirmish AI for Byte Tactics ([ozgb/btm](https://github.com/ozgb/btm)): it expands, holds a reserve, times its waves by an odds gate, counters the enemy's unit mix, protects its commander, and uses air, transports and ferries. Personalities (Balanced, Rush, Turtle, Tech Rush) and difficulty tiers are data.

## Provenance

This is a clean-room implementation, under MIT. Its behaviour comes from a written spec (`SPEC.md`) of RWE-B4's "RWE AI", which is GPLv3. Someone who read RWE wrote the spec. The code was written only from the spec, by someone who never read RWE's source. No RWE code is in this repository. The spec leaves out RWE's port of the original TA AI on purpose.

## Design

- **Game access through a trait.** The AI reads the game only through a fog-honest `GameView` trait and acts only through `Orders`. An adapter to btm's `ai` module interface comes once that interface exists.
- **Headless test sim.** A small TA-like simulation in this repo is for behaviour tests only. It doesn't measure real strength: that needs the btm engine and an arena.
- **Deterministic.** The AI makes no hidden random draws and reads no wall clock, so it is safe in lockstep.

## Using it

A host gives each AI seat three things:

- **`GameView`** (`view::GameView`): what the seat may know, honest to the fog. That covers its own units and economy, the map and the unit definitions, and the enemies it sees now or saw last.
- **`FogLeaks`** (`view::FogLeaks`): the few things the spec lets the AI read through the fog (SPEC §1.4). These are placement tests that see every building, buildings near a point, and feature contents in explored cells. They are kept apart so that each leak is visible and can be audited.
- **`Orders`** (`view::Orders`): where the AI's orders go. Each `Order` is a unit, a `Command`, and Immediate or Queued. `OrderRecorder` collects them for one tick.

```rust
use btm_ai::brain::Ai;
use btm_ai::personality::{resolve, Difficulty, Faction, Registry};
use btm_ai::view::OrderRecorder;

let reg = Registry::builtin();
let tuning = resolve(reg.get("Rush").unwrap(), Difficulty::Standard, Faction::Arm, &[])?;
// `seed` is the one value the seat draws from the game's random stream.
let mut ai = Ai::new(player, &tuning, seed)?;

// Every simulation tick, exactly once:
let mut out = OrderRecorder::new();
ai.tick(&view, &leaks, &mut out)?;
host.submit(player, out.take()); // applied after the command-buffer delay (7 ticks)
```

`resolve` builds the knobs in this order: difficulty tier, then faction defaults, then the personality, then any arena overrides (SPEC §6.1). The host must apply the cheats a tier declares itself (`tuning.cheats`, `knobs.cheat.*`): full vision and an income factor. `sim::Game` shows a complete host loop.

### Personalities

| Name | Play |
|---|---|
| Balanced | The tuned default, with no knobs of its own. |
| Rush | Few towers, a small first wave of raider kbots, raids, no tier two. |
| Turtle | Many walled towers, outposts and a large first wave. |
| Tech Rush | Goes to tier two early, with fewer towers and a larger wave. |
| Easy, Medium, Hard | The default personality at that difficulty. |

A personality is a small `knob=value` text file (SPEC §6.5). `Registry::load_dir` adds a folder of them to the built-ins.

### Difficulty tiers

| Tier | What changes (SPEC §6.2) |
|---|---|
| Idle | Builds the seat but never issues an order. |
| Easy | Fewer buildings, bigger waves, slower cadence, 60 orders/min micro cap. |
| Standard | The spec defaults, 120 orders/min micro cap. |
| Hard | More buildings, smaller waves, faster cadence, no micro cap. |
| Brutal | Like Hard, plus full vision and a 1.25 income factor (cheats). |

## The arena

The tuning arena (SPEC §8) plays paired games. A control game and a tuned game share a seed, and the report gives the fraction of tuned games that were better, with a Wilson interval and an exact sign test. `sim::SimMatch` runs those games in the headless sim:

```sh
cargo run --release --example arena -- [A] [B] [N] [SECONDS] [land|water] [FIRST_SEED]
# defaults: Balanced Rush 16 480 land 1
```

This plays N pairs: half on ARM mirrors and half on CORE mirrors. Each pair is A against A, then B in the scored seat against A, on the same seed. `cargo run --release --example tick_time` times `Ai::tick` over one AI-against-AI game.

The sim is a small stand-in for the game. Its results show how the AI copes with the sim. They do not measure strength in the real game.

Where SPEC.md could be read more than one way, `docs/spec-readings.md` lists the reading taken and why.

## Status

Work in progress.
