# btm-ai

A skirmish AI for Byte Tactics ([ozgb/btm](https://github.com/ozgb/btm)): it expands, holds a reserve, times its waves by an odds gate, counters the enemy's unit mix, protects its commander, and uses air, transports and ferries. Personalities (Balanced, Rush, Turtle, Tech Rush) and difficulty tiers are data.

## Provenance

This is a clean-room implementation, under MIT. Its behaviour comes from a written spec (`SPEC.md`) of RWE-B4's "RWE AI", which is GPLv3. Someone who read RWE wrote the spec. The code was written only from the spec, by someone who never read RWE's source. No RWE code is in this repository. The spec leaves out RWE's port of the original TA AI on purpose.

## Design

- **Game access through a trait.** The AI reads the game only through a fog-honest `GameView` trait and acts only through `Orders`. An adapter to btm's `ai` module interface comes once that interface exists.
- **Headless test sim.** A small TA-like simulation in this repo is for behaviour tests only. It doesn't measure real strength: that needs the btm engine and an arena.
- **Deterministic.** The AI makes no hidden random draws and reads no wall clock, so it is safe in lockstep.

## Status

Work in progress.
