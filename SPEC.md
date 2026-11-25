# Skirmish AI: behavioural specification

This document says what the skirmish AI module must do. It does not say how
to structure the code. It describes an opponent that plays Total
Annihilation-style games through the same order stream a human uses, and it
is complete enough to rebuild an AI of a known strength. Every number here is
a tuned value or a measured threshold, and the implementer should treat them
as the starting tuning.

Conventions:

- **Time.** The simulation runs at 30 ticks per second. "N s" means N × 30
  ticks.
- **Distance.** Distances are in world units. A heightmap tile is 16 units,
  and a fog/vision cell is 32 units (2 × 2 tiles). Distances are flat x/z
  distances unless the text says otherwise.
- **Knobs.** A knob appears in `monospace` the first time it is used, with its
  default in brackets. Appendix A lists every knob.
- **Roles and ids.** Units are named by role ("the side's first kbot lab"),
  followed by the stock ARM / CORE unit ids in brackets. Appendix B is the
  role table. A rule that names a role does nothing if the side has no unit in
  that role.
- **Priority.** "First match wins" means the rules are tried in the order
  given, and the first rule whose conditions hold is the one acted on.

---

## 1. Inputs: what the AI may read

### 1.1 Principle

The AI must see no more than a human player in the same seat, unless the
full-vision cheat (`cheat.full_vision` [off]) is set. Brutal difficulty sets
it. All knowledge of the enemy goes through a **contact memory** (§1.3).
Other parts of the AI read enemy information only from that memory, or by
asking whether a remembered contact can be seen *now*. The AI can read the
following freely:

- its own units;
- public map data;
- unit and weapon definitions, which are game data;
- its own economy.

### 1.2 Queries made of the game

| Query | Used for | Fog status |
|---|---|---|
| Own player: metal and energy stock, storage capacity, stall flags, last second's income and desired consumption for both resources | Every economic decision | Own data |
| Own units: type, position, health, build progress, order queue, factory queue and factory state, carried units, on/off state, the frame a builder is working on | Everything | Own data |
| Unit definitions: cost, build time, worker rate, health, speed, footprint, movement class (slope and depth limits), flags (builder, mobile, flies, commander, hovers, floats, transport capacity and size, can-be-transported, on/off, makes metal, extracts metal, energy use, radar, air base), build menus | Classification, estimates, siting | Game data |
| Weapon definitions: range, damage against the default armour, burst, reload, anti-air flag, lobbed/dropped flags, underwater flag, energy per shot | Threat, kiting, d-gun, line of fire | Game data |
| Build menus: which builder can build which type | Every build choice. A builder with no known menu may build anything. | Game data |
| Terrain: height at a point, sea level, map bounds, metal value per cell, map-wide surface metal, geothermal vent features | Siting, reachability, map intel | Public map data |
| Declared start positions | Scouting, ferry need, the island test | Public (lobby preview) |
| Placement legality at a point (footprint and occupancy) | Siting | See §1.4 |
| "Can I see unit U now?" and "Is point P in my line of sight?" | Contact memory | Fog-honest |
| "Is point P under my radar?" | Radar warning | Fog-honest |
| Per-cell "currently visible" grid | Staleness for scouting | Fog-honest |
| Explored flag per cell | Reclaim and vent filters | Fog-honest |
| Features (wrecks, rocks, trees): position, metal, energy, reclaimable flag | Reclaim | See §1.4 |
| Players' alive/dead status and team | Choosing the enemy to fight; excluding allies | Public |
| A unit's "immune" flag (missions) | Immune units are never contacts | Mission data |
| Weapon-versus-target legality ("can this unit's weapons hit that target?") | Focus fire, raids, the navy | Asked only of contacts seen now |

### 1.3 Contact memory (the fog-honest enemy model)

Each tick, the contact memory goes through these steps:

1. **Forget.** A remembered contact is forgotten only when the AI is looking
   at the contact's last known position and the unit is not visible there,
   whether it moved or died. A unit that dies out of sight stays on the AI's
   map until something goes and looks. With full vision, a contact is
   forgotten the moment it dies.
2. **Record.** Every non-allied, non-immune, living enemy unit that the AI can
   currently see is recorded or refreshed. The record holds:
   - id, type and owner;
   - last known position and time last seen;
   - time first seen (kept across refreshes);
   - building or mobile;
   - armed (has any weapon);
   - aircraft;
   - commander;
   - builder;
   - whether it was still a frame when seen.
   An ally (same non-empty team) is never a contact.
3. **"Standing" rule.** Other parts of the AI ask whether a contact is still
   standing. If the AI is looking at the last known spot, the answer is yes
   only if the unit is alive and visible, and the live unit may then be read.
   If the AI is not looking, the answer is yes, but only the remembered
   fields may be used. "Seen now" means it was refreshed this tick (always
   true with full vision). Any order aimed at a unit needs the target seen
   now.
4. **Derived facts**, recomputed each tick from the memory:
   - **Enemy base position.** Kept per enemy player: the centroid of that
     player's remembered buildings. If none of its buildings are remembered,
     its commander's last position is used.
   - **Focus enemy** (`army.one_enemy` [on]). This is the living enemy player
     whose base position is nearest our home point. The choice changes only
     when the current focus is gone, or another enemy is nearer by more than
     `army.enemy_switch_margin` [600]. While a focus exists, "enemy base" and
     "enemy commander position" refer to that player.
   - **Air threat.** True while any enemy aircraft is remembered, and for 300
     s after the last sighting. Separately, the peak number of armed enemy
     aircraft remembered at one time is kept for the game.
   - **Army values.** Our army's metal (§5.1) and the known enemy's metal,
     split into mobile armed ground and mobile armed air.
   - **Role histogram** (§5.3).
   - **Intruders**, the sector threat memory and the radar warning (§5.4).
   - **Besieged factories.** A factory counts as harassed if one of our
     *frames* died within `build.harass_radius` [400] of it in the last 30
     s. A harassed factory with an armed remembered enemy (aircraft included)
     within 400 is *besieged*.

### 1.4 Where the reference AI is not fog-honest

These are leaks in the reference AI. A clean implementation may close them,
but closing some of them may cost a little strength, so measure.

- **Placement tests see all buildings.** The legality and occupancy tests,
  and the clearance lanes kept around buildings and factories (§4.6), take
  account of every building of every owner, seen or not. The extractor
  search also keeps clear of enemy factories it has never seen.
- **Tier-two extractor upgrade.** The crowding test reads every building near
  our own extractor, of any owner.
- **Features** are filtered by "explored", not "visible now". The AI knows the
  current contents of wrecks in any area it has ever explored. Its own
  wreck-site reclaim has no explored test at all.
- **Geothermal vents.** The buildability test reads true occupancy, so a vent
  the enemy took out of sight is rejected.
- **Radar warning.** It reads whether an unseen radar dot is mobile, which a
  player's radar does not tell them. It also appears to count allied mobile
  units as contacts.
- **Metal patches** are all known from the start. That is deliberate: a
  player sees every metal spot on the map.
- The doc's claim that the AI "only sees what fog allows" is true of all
  *enemy unit* knowledge, apart from the radar detail above. It is not true
  of placement or of features.

---

## 2. Outputs: orders and cadence

### 2.1 Orders the AI issues

All orders go through the normal player command stream, either Immediate
(replacing the queue) or Queued.

| Order | Issued by |
|---|---|
| Build type T at position P | Builders (§4) |
| Factory queue: add one of type T; remove N (negative count) | Factories (§4.4) |
| Move to P | Army, navy, air, scouts, builder retreat, transports, mend spots, rally. A move to the unit's own position is used as "stop". |
| Attack unit | Army, air, navy, anti-air, commander |
| Attack ground position (attack-move: fight whatever is met on the way) | Waves |
| Patrol through a list of points (first point Immediate, the rest Queued) | Standing air patrols |
| Guard unit | Anti-air escorts; builders assisting a busy factory |
| Repair unit | Damaged structures, unfinished frames, assisting frames, mending units, the commander, the commander returning to its frame |
| Reclaim feature or unit | Wreck harvest, field reclaim, own wreckage, clearing a wreck before a rebuild, taking down an extractor for an upgrade |
| Fire the commander's special weapon (d-gun) at a unit | Commander |
| Load unit; unload at P | Transports |
| Stop | Transports calling off a trip |
| Switch on/off | Metal makers |

**Capture** is never issued. Self-destruct, sharing resources and alliance
changes are never issued either.

### 2.2 Cadence

The AI is ticked every simulation tick and decides on fixed intervals of game
ticks. Wall-clock time is never used.

| Pass | Interval (Standard) | Easy | Hard / Brutal |
|---|---|---|---|
| Economy sampling, contact memory, phase machine | every tick | | |
| Influence grid rebuild (`cadence.threat_grid_ticks`) | 30 ticks | 30 | 30 |
| Reachability re-home | first chance, then every 20 s | | |
| Build planning (`cadence.plan_ticks`): factories topped up, ONE idle builder given a job | 30 ticks | 60 | 20 |
| Builder-safety sweep, commander-repair dispatch, defence attack watch | 15 ticks | | |
| Army, air and transport (`cadence.tactics_ticks`) | 15 ticks | 30 | 10 |
| Scouts (`cadence.scout_ticks`) | 60 ticks | 120 | 45 |
| Metal makers | every tick, but at most one command per maker per second | | |
| Radar warning sample | 1 s | | |

The first tactics pass happens on the 15th tick, not tick 0.

### 2.3 The micro limiter

After the army and air passes, their move, attack, patrol, d-gun and guard
orders are rate-limited. Repairs, builds, factory and transport orders pass
untouched.

| Difficulty | Limit |
|---|---|
| Easy | 60 orders per minute |
| Standard | 120 orders per minute |
| Hard, Brutal | no limit |

How the limit works:

- A token bucket with a burst of 4 that refills pro rata each tick and starts
  full. A sliding 60 s window total also may not exceed the rate.
- A unit's orders in one tick are admitted or refused as a group.
- Units are served in this order: units never served before first, then the
  least recently served.
- Once one unit is refused, every later unit that tick is refused too.
- Refused orders are dropped. The next pass regenerates them from fresh state.

### 2.4 Final reservation gate (missions only)

When the host supplies a per-type limit table (campaign missions), every
produced build or queue order of a forbidden or over-limit type is removed
before the orders are sent. Counts include:

- existing units;
- frames;
- queued factory entries, less the unit a factory is currently holding.

In skirmish the table is empty and the gate does nothing.

---

## 3. Architecture as behaviour

### 3.1 The per-tick pipeline

The AI keeps one **world model** per seat, and the steps below read and write
it in this order every tick. Each later step may rely on earlier steps having
run this tick.

1. **Own-state sampling.** This step does the following:
   - classifies every own unit (§3.2);
   - counts completed units, and "total" units per type (completed + frames +
     build orders not yet placed);
   - computes the metal committed per second by builders and factories
     working on frames: the remaining metal of each frame divided by its
     remaining build time;
   - records losses: buildings and mobile units that vanished. It keeps the
     last 8 building losses for 60 s and the last 16 unit losses for 30 s,
     noting whether each was a frame;
   - records own wreck sites: deaths within 1200 of home, at most 32 sites;
   - finds orphaned building frames (frames no builder is attending);
   - finds idle builders (no orders), plus builders whose only order is guard;
   - finds the commander. The **home point** is the commander's first
     position, and it is never revised. Before a commander is seen, home is
     the first factory, else any own unit.
   - computes army size = land combat units, minus one if a combat unit is
     borrowed as the scout.
2. **Contact memory** (§1.3).
3. **Influence grid** (§3.3), when due.
4. **Reachability** (§3.4), when due.
5. **Phase machine** (§5.1).
6. **Metal makers** (§4.9).
7. **Build planner** (§4). It records whether it gave the commander an order
   this tick, so that the army pass does not overwrite it.
8. **Scouts** (§5.10).
9. **Transports** (§5.9).
10. **Army and navy** (§5).
11. **Air** (§5.8).
12. **Micro limiter** (§2.3).
13. **Mission reservation gate** (§2.4).

The planner, army and air are not peers that negotiate. Coordination goes
through flags in the world model, each written by one step and read by later
ones:

- the build-site guard request;
- the ferry rally point;
- the borrowed scout;
- ferry passengers, whom other passes leave alone;
- "commander tasked", "commander fleeing" and "builder sheltering";
- the attack group, contact time and wave centre.

### 3.2 Own-unit classification

First match wins:

1. An immobile builder is a **factory**.
2. A mobile non-builder transport is a **transport**.
3. A mobile naval combat unit is a **hull** (navy).
4. A mobile scout type, or an unarmed non-builder aircraft, is a **scout**.
5. A builder is a **builder**, which may be idle or assisting.
6. Mobile anti-air of the side's anti-air kbot type is **anti-air**.
7. A mobile, armed ground unit that can attack is a **combat unit**.

Aircraft with weapons are not combat units. The air pass owns them. Units
booked as ferry passengers are left out of the idle, anti-air and combat
lists. Units held by a mission script are counted but never ordered.

### 3.3 Influence grid

The grid is at vision-cell resolution (32 units) and is rebuilt from the
contact memory every 30 ticks. It has four layers:

- **Ground threat.** For each remembered contact with damage, add its summed
  dps to every cell within its longest weapon range (a disc of cells). Summed
  dps is, over all of its weapons, default damage × max(1, burst) / max(0.1,
  reload).
- **Anti-air cover.** For each contact with an anti-air weapon, add 1 to every
  cell within its anti-air range.
- **Economic value.** The metal cost of each remembered enemy building, added
  to its cell.
- **Staleness.** Each rebuild, a cell that is visible now resets to 0, and
  every other cell goes up by 1. The count starts at 100000, meaning never
  seen. With full vision every cell is 0.

Queries made of the grid:

- **Best attack cell.** Among cells holding enemy buildings, the score is
  value − ground threat × `army.threat_aversion` [1]. The best cell within
  `army.target_base_radius` [900] of the enemy base is preferred; failing
  that, the best cell anywhere.
- **Best scout cell.** The score is min(staleness, 3000) / max(1, distance),
  subject to an accept test.
- **Ground threat in a radius**, as a sum.

### 3.4 Reachability and map intel

**Map intel** is computed once at load from public data:

- **Water fraction**: the share of heightmap cells below sea level. The map
  character is **Land** below 0.12, **Mixed** below 0.40, and **Water**
  otherwise.
- **Water bodies**: 4-connected regions of submerged cells.
- **Shipyard sites**: every tile whose 8 × 8 footprint has a water depth of
  30-255 throughout. A site is "open" when a ring 6 tiles wider is also at
  least 30 deep.
- The start positions.

**Reachability layers.** Each layer is a 4-connected flood fill of "standable"
tiles for one movement class (footprint, slope and depth limits). The fill is
recomputed only when the movement class changes; otherwise the layer is just
re-homed.

- **Ground layer.** The mover is the side's level-one constructor (or the
  commander if there is none), and the layer is homed on the home point. If
  *no* own factory is reachable on it, it is re-homed on the first walkable
  factory, which becomes the **ground home**. This fixes bases the commander
  built where its kbots cannot leave; the commander wades deeper and climbs
  steeper than a constructor.
- **Commander layer.** The commander's own class. Used for the commander's
  build sites (`cmd.own_reachability` [on]).
- **Naval layer**, built on non-Land maps only. The mover is the sea
  transport, else the destroyer, submarine, scout ship or construction ship,
  in that order. It is homed on our shipyard, else on the shipyard site
  nearest home.
- **Hover layer**, for data with a hovercraft (non-Land maps, at least 2
  starts).

If a layer's home tile is not standable, its four neighbours' regions are
claimed instead. A query point is the unit's footprint top-left tile. A layer
that has not been built answers "reachable" to everything.

Derived flags:

- **Unreachable ground exists**: walkable tiles exceed reachable tiles by more
  than 64.
- **Land route to enemy**: any other declared start position is reachable on
  the ground layer. It is unknown on maps with fewer than 2 starts, and
  unknown counts as yes.
- **Hover route to enemy**: the same test on the hover layer.

Known weakness: the fill is 4-connected while the pathfinder moves in 8
directions, so narrow diagonal passes can read as cut.

---

## 4. Economy and build

### 4.1 Shared economy predicates

These thresholds are fixed constants. A fraction is stock / storage capacity.

| Name | Holds when |
|---|---|
| metal short | metal stalled, or metal stock < 10% of storage |
| metal full | metal stock ≥ 80% |
| metal pegged | metal stock ≥ 90% |
| energy short | energy stalled, or energy stock < 25% |
| energy wanted | energy stalled, or energy stock < 50% |
| energy rich | energy stock ≥ 80% and energy income > energy demand |
| air matters | map character is not Land, or unreachable ground exists |
| tech income | metal income ≥ `tech.min_income` [10] |
| tier worth it | the value ratio ≥ `tech.min_value_ratio` [1.5]. The value ratio is the side's tier-two assault kbot's (health × best-weapon dps / metal) divided by the better of the raider's and rocket kbot's. It is computed once. |
| naval fleet target | 0 if `navy.fleet` [6] ≤ 0. Water map: `navy.fleet`. Mixed map: max(1, fleet / 2) if unreachable ground exists, else 0. Land map: 0. |
| capacity short | metal demand × `eco.capacity_ratio` [1.25] < metal income, continuously for `eco.capacity_s` [20] (needs `eco.capacity_spend` [on]) |

**Affordability.** An item is affordable if either:

- metal stock ≥ 90% of storage; or
- stock + (income − committed spend) × (build seconds + extra seconds) ≥ the
  metal still owed.

Build seconds = remaining build time / builder worker rate / 30.

### 4.2 Builder assignment loop

Each planning pass:

1. Factories are topped up (§4.4).
2. **Exactly one** idle builder is chosen, round-robin over the idle list
   (which includes assisting builders), with a cursor that persists between
   passes.
3. The chosen builder is given at most one job, first match wins:
   1. A fleeing commander gets nothing. So does a builder in its shelter
      window (§5.7).
   2. **Dropped-order check.** If this builder was given a build order within
      `build.failed_site_s` [120], and nothing of ours stands within 48 of
      that site, the site's cell is marked *failed* for 120 s. Raided
      extractor sites (dry extractors in the recent losses) are remembered for
      `def.raid_memory_s` [300].
   3. **Context.**
      - The commander plans on its own reachability layer; other builders on
        the ground layer.
      - A builder is **at base** if it flies, or its position is reachable.
      - The anchor is home if the builder is at base, else the builder's own
        position. A site must be on the builder's side of reachability.
      - Water structures are exempt from the reachability test: tidal, sonar,
        torpedo launcher, floating maker, underwater storages and underwater
        fusion.
   4. **Repair** a damaged structure (§4.11), if the builder can repair.
   5. **Resume an orphaned frame**, provided it is reachable, not exposed
      (§5.7), and either an economy type or affordable.
   6. **Assist an advanced-lab frame** that is under construction and
      reachable, with no affordability test.
   7. **Mend a damaged mobile unit** (§4.11).
   8. **Field reclaim**, then **own-wreck reclaim** (§4.12).
   9. **Extractor upgrade in progress** (§4.3, last row of the site table).
      If this builder owns the upgrade, it re-issues the reclaim while the old
      extractor stands, then builds the tier-two extractor at the site.
   10. **Walk the priority list** (§4.3) and take the first entry that is
       affordable and has a site (details below).
   11. **Nothing taken:**
       - **Commander assist.** The commander repairs the nearest own frame
         within `build.commander_assist_radius` [800] that is reachable and
         within `build.commander_leash` [1200] of home. Only when not saving.
       - **Harvest reclaim.** Applies to builders at base, and to ships. It
         runs even while saving.
         - The centre is home (or the ship's position), radius 1200.
         - Eligible features: reclaimable, on explored ground, not under
           enemy guns, with metal > 0 (or energy > 0 when energy is wanted).
           Ships only take submerged features.
         - Metal-bearing features first, then the nearest.
       - **Factory assist.** The builder guards the nearest factory with a
         non-empty queue. The factory must be of the same land/water nature as
         the builder; flyers may assist any. Only when not saving.
       - Otherwise the builder stays idle.

Throughput is therefore one new builder job per planning pass for the whole
AI. This is a load-bearing tuning fact.

**Walking the list.** For each wanted type:

- **Economy types skip the affordability test.** These are the dry extractor
  and the land metal maker, always, and solar or tidal when energy is wanted.
- **Saving.** Every other type that is not affordable is tested again with an
  extra window:
  - `eco.save_tech_s` [240] for the advanced lab, seaplane plant, hover plant,
    advanced shipyard, fusion, underwater fusion, tier-two extractor, heavy
    plasma tower, heavy laser tower, advanced air plant, repair pad and
    torpedo launcher, and for the first air plant while air is worth it
    (§4.4);
  - `eco.save_s` [60] for everything else.

  If the item is affordable within that window, the AI is now **saving for
  it**. It continues down the list, but only economy types may be taken until
  the saved-for item is built. Otherwise the item is skipped. Saving does not
  depend on a site existing; tying it to one measured worse.
- **Site choice** follows §4.6.
- **Tower worth.** A light, AA, heavy or plasma tower must have metal cost ≤
  income × (`def.payback_s` [90] + covered extractors ×
  `def.payback_per_mex_s` [45]). Base towers cover 0 extractors. The test is
  waived:
  - when urgent (an enemy near base, or a raided outpost);
  - for walls and rebuilds;
  - when the payback knob is ≤ 0;
  - when income is 0.
- **Contested site** (any type but extractors). A site is contested if a
  remembered armed ground enemy is within `build.contested_radius` [450], or
  if it is a non-rebuild site beyond `army.defend_radius` [900] from home with
  a recent loss within 450. A contested site is marked failed and skipped.
  Failed cells are skipped, and so are sites under enemy guns (§4.6).
- **Issue the job.** If the site is blocked by a reclaimable wreck, the
  builder reclaims the wreck immediately and the build is queued behind it.
  Saving is cleared. A build-site guard may be requested (§5.6).

### 4.3 The build priority list (the hard-coded build order)

The list is rebuilt for the chosen builder every time. Entries the builder has
no menu button for are removed, and duplicates keep their first position.
**The order of the list is the strategy.** After the list is built:

- every type recently lost (building losses, most recent first) is moved to
  the front;
- missions may then stable-sort the list by a per-type weight.

**Opening.** There is no script. The opening is just the top of the list
running against a fresh base. With defaults the AI builds:

1. 3 extractors;
2. 4 solars (solars first if energy is stalled);
3. the first kbot lab;
4. then whatever the list wants next.

In stock unit ids:

| Side | Opening (commander) |
|---|---|
| ARM | ARMMEX ×3, ARMSOLAR ×4, ARMLAB, then ARMLLT ×2 / ARMRAD / ARMRL ×1 / ARMVP (land maps) or ARMAP (water or broken maps), with more ARMMEX and ARMSOLAR between |
| CORE | CORMEX ×3, CORSOLAR ×4, CORLAB, then CORLLT ×2 / CORRAD / CORRL ×1 / CORVP or CORAP, likewise |

The lab immediately builds 1 constructor (ARMCK / CORCK). The second and later
constructors are pushed to expand (rule E below).

**At-base list, in priority order:**

| # | Wants | Condition |
|---|---|---|
| 0 | solar (+ tidal if naval fleet target > 0) | energy stalled |
| 1 | extractor (dry; underwater too if the map has submerged patches) | extractors < `open.mex` [3] |
| 2 | solar | solars < `open.solar` [4] |
| 3 | first kbot lab | timed opening only (§4.13): time ≥ `open.factory_by_s`, quiet base, no lab, no shipyard |
| 4-5 | extractor | timed opening expansion rules (§4.13) |
| 6 | extractor | metal short and extractors < `eco.mex` [8] |
| 7 | first kbot lab | none owned, unless land factories are pointless |
| 8 | shipyard (early) | `navy.early_shipyard` [on], naval fleet target > 0, shipyards < `navy.shipyards` [2], (a lab exists or land factories are pointless), solars ≥ 4, dry extractors ≥ 3 |
| 9 | kbot lab (late position) | land factories pointless but the lab is not, none owned |
| 10 | AA tower (reactive) | air threat and AA towers < `def.reactive_aa_towers` [3]; no metal gate |
| 11 | metal maker, then floating maker | lab exists, energy rich, (metal short or a surplus maker is allowed), makers below the ceiling (§4.9) |
| 12 | radar | radars < `build.radar` [1] and solars ≥ 4 |
| 13 | rebuild a lost tower (§4.8) | a rebuild plan exists, and (not metal short, or an enemy is near base) |
| 14 | light laser tower | not metal short, fewer than `def.towers` [2], lab exists |
| 15 | next wall or AA tower of a fortification plan (§4.8) | a plan exists, and (not metal short, or an enemy is near base) |
| 16 | light laser tower at an outpost (§4.7) | an outpost plan exists, lab exists, (not metal short, or the outpost was raided) |
| 17 | AA tower (standing) | not metal short, lab exists, AA towers < (3 if air threat, else `def.base_aa_towers` [1]) |
| 18 | geothermal | the map has vents, a lab or shipyard exists, geothermals < vents |
| 19 | advanced kbot lab | `tech.enabled` [on], tier worth it, tech income, lab exists, advanced labs < `build.adv_labs` [1] |
| 20 | hover plant | hover worth it (§5.9), a land factory exists, below `build.hover_plants` [0] |
| 21 | vehicle plant (early) | `build.vehicle_plant_first` [on], air does not matter, land factories not pointless, not metal short, a factory exists, opening energy (solar + tidal ≥ 4), vehicle plants < `build.vehicle_plants` [1] |
| 22 | air plant | a factory exists, air plants < `build.air_plants` [1], opening energy, (air matters or dry extractors ≥ 8), the early vehicle plant is done if rule 21 applies, (air matters or radar is done) |
| 23 | advanced air plant | tech on, tech income, air worth it, an air plant and an air constructor exist, below `build.adv_air_plants` [1] |
| 24 | air repair pad | an advanced air plant exists, below `build.repair_pads` [1] |
| 25 | solar | solars < `eco.solar` [10], unless `eco.solar_on_demand` [on] and energy rich |
| 26 | extractor (dry) | dry extractors < 8 |
| 27 | vehicle plant (late) | not metal short, land factories not pointless, air plant done, below target |
| 28 | extra factory | capacity short, not metal short, a factory exists. If land factories are pointless: shipyard while < shipyards + `eco.capacity_factories` [1]. Else vehicle plant while < target + 1. Else lab while < 2 (unless the lab is pointless). |
| 29 | shipyard (late) | naval fleet target > 0, shipyards < 2 |
| 30 | tidal | naval fleet target > 0, tidal < `eco.tidal` [6] |
| 31 | sonar | naval fleet target > 0, below `navy.sonar` [1] |
| 32 | torpedo launcher | naval fleet target > 0, an enemy ship seen, below `navy.torpedo_launchers` [1] |
| 33 | advanced shipyard | naval fleet target > 0, a shipyard exists, below `navy.adv_yards` [1], metal income ≥ `navy.tech_income` [15] |
| 34 | metal storage, then energy storage (land kind, then underwater kind) | a factory exists. Metal storage: below `eco.metal_storage` [2] and (a tier-two factory stands with no store yet, or metal pegged). Energy storage likewise with `eco.energy_storage` [2] and energy ≥ 90%. |
| 35a | heavy laser tower | an advanced lab exists, under attack (enemy near base or recent losses), below `def.heavy_towers` [2] |
| 35b | tier-two extractor | an advanced lab exists, tech income |
| 35c | advanced radar | an advanced lab exists, below `build.adv_radar` [1] |
| 35d | fusion | an advanced lab exists, below `eco.fusion` [1], and (energy short, or metal full, or tech income with no fusion yet) |
| 35e | heavy laser tower | an advanced lab exists, not metal short, below 2 |
| 35f | heavy plasma tower | an advanced lab exists, tech income, below 2 |
| 36 | extra kbot lab | metal full, tier two not still wanted, labs < 1 + `eco.extra_labs` [1] |
| 37 | surplus expansion | `eco.surplus_expand` [on] and metal full. Allowed = min(`eco.surplus_factory_cap` [4], 1 + floor(income / `eco.surplus_factory_step` [20])). (a) Advanced lab if income ≥ 20 and none exists, ignoring the tech knob and the value ratio. (b) Naval maps: shipyards up to the allowed count, plus an air plant if none exists. (c) Otherwise: labs up to the allowed count, then vehicle plants up to the allowed count. |
| 38 | extractor (dry) | always; no ceiling |
| 39 | underwater extractor | the map has submerged patches |

**Rule E: expansion constructors.** The dry extractor is pushed to the very
front, ahead of everything including loss promotion, when all of these hold:

- the builder is the side's level-one constructor, not the commander, and at
  base;
- the number of living constructors with a lower unit id is ≥
  `build.constructors` [1];
- at least one free deposit exists on our side (§4.5).

The effect is that the first constructor builds the base and every later one
expands.

**"Land factories pointless".** This is a mission and island option,
`build.mission_isolated_sea_air` [0, off]. It holds when all of these are
true:

- the knob is > 0;
- there is no land route to the enemy;
- the naval fleet target is > 0 and a shipyard role exists;
- we own no transport;
- the base is quiet.

At a knob value of 2 the lab is also pointless and is dropped.

**Stranded-builder list.** A builder that is not at base, or any construction
ship (unless `navy.ship_builders_plan_base` [off]), uses this short list
instead. Recent-loss promotion is not applied.

1. Advanced shipyard (as rule 33).
2. Seaplane plant: surplus expansion on, metal full, no air plant, an
   advanced shipyard exists, below 1.
3. Underwater fusion: an advanced shipyard exists, income ≥ 15, below 1, and
   (no reactor yet, or energy short).
4. Underwater storage (as rule 34).
5. Extractor: dry first, then underwater.
6. Solar.

**Tier-two step, summarised:**

- The advanced lab is wanted when tech is on, income ≥ 10, the value ratio ≥
  1.5 and a lab stands.
- A full store with income ≥ 20 forces it regardless (rule 37a).
- While it is still wanted, the extra-lab rule is suppressed.
- Every idle builder, the commander included, assists its frame.
- Once it stands, tier two is spent in this order: heavy laser tower if under
  attack, tier-two extractor, advanced radar, fusion, heavy laser tower, heavy
  plasma tower.

### 4.4 Factory production

Every planning pass, for each own factory in id order:

1. **Besieged.** Cancel every queued entry and stop
   (`build.notice_harassment` [on]).
2. **Stalled head** (`build.drop_stalled_head` [on]). Watch the head of the
   queue: its type and the frame it is on. The watch restarts on a change of
   type or frame, or if progress goes backwards. If the watch has lasted ≥
   `build.stall_s` [60], progress since it started is below max(1 tick, 1% of
   build time), and the head is unaffordable (here the 90%-full override does
   not apply), remove every queued unit of that type. The clock restarts
   whenever progress reaches the 1% floor or the head becomes affordable.
   Suspended while the tier-two reserve holds.
3. **Queue depth.** Queue something only if the queue has fewer than
   `build.queue_depth` [1] entries; with the default, only when it is empty.
   Depth 2 measured as a regression.
4. Choose one unit (tables below) and queue +1.

**Tier-two reserve** (`eco.t2_reserve` [on]). Items are "owed" when:

- an advanced constructor is complete and no tier-two extractor is; or
- an advanced constructor exists (or an advanced construction sub with an
  underwater fusion option exists) and no reactor of either kind is complete.

While anything is owed, the reserve **holds** if all of these are true, and
while it holds only builders are queued:

- time since the first moment anything was owed < `eco.t2_reserve_max_s`
  [480]. The start time is latched once per game and never reset; the
  reference AI may have this by accident.
- The base is quiet.
- Army plus hulls ≥ `eco.t2_reserve_min_army` [6].

**Share picking.** Each candidate has a share (knob plus counter lean). Pick
the candidate with the smallest owned / share ratio, compared by
cross-multiplication so there are no divisions; the first listed wins a tie.
Zero-share candidates are skipped.

**Kbot lab** (also any unrecognised factory type):

1. A constructor while below the constructor target. The target is
   `build.constructors` [1], plus:
   - `eco.capacity_constructors` [2] while capacity is short;
   - min(`build.expansion_constructors` [2], free deposits on our side /
     `build.deposits_per_constructor` [4]);
   - `fort.extra_constructors` [1] while walls are owed under walling every
     tower;
   - with the timed opening, at least 1 after `open.constructor_by_s`.
2. Anti-air kbot (ARMJETH / CORCRASH) if air threat and below `def.mobile_aa`
   [2].
3. If the island land-army cap binds, nothing.
4. Otherwise by share among raider kbot (ARMPW / CORAK), rocket kbot
   (ARMROCK / CORSTORM) and artillery kbot (ARMHAM / CORTHUD). The shares are
   `mix.lab_raider` [2], `mix.lab_rocket` [1] and `mix.lab_artillery` [0],
   plus the counter lean (§5.3), before faction defaults (§6.3).

**Vehicle plant:**

1. A scout vehicle (ARMFAV / CORFAV) while below `scout.vehicles` [1].
2. Otherwise by share among light tank (ARMFLASH / CORGATOR) at
   `mix.vp_light_tank` [1], missile truck (ARMSAM / CORMIST) at
   `mix.vp_missile` [0] and medium tank (ARMSTUMP / CORRAID) at
   `mix.vp_medium_tank` [0], plus the lean.

**Advanced kbot lab:**

1. An advanced constructor (ARMACK / CORACK) while below
   `build.adv_constructors` [1].
2. Otherwise the tier-two assault kbot (ARMZEUS / CORCAN), with no ceiling.

**Air plant**, first match:

1. An air transport if the ferry is overdue (§5.9) and an air transport is
   wanted.
2. A scout plane below `air.scout_planes` [1].
3. An air transport if wanted. It is wanted when transport is wanted or the
   ferry priority is raised, the AI has cargo, it is below `air.transports`
   [1], and a sea lift is not preferred.
4. A fighter if air threat, armed enemy air has been seen, and fighters <
   `air.fighters` [2].
5. An air constructor below `build.air_constructors` [1].
6. A fighter below `air.patrol_fighters` [2].
7. A fighter if air threat and fighters < the fighter target. The fighter
   target is the peak count of armed enemy air, clamped to [2,
   `air.fighters_max` 8].
8. A bomber below `air.patrol_bombers` [2].
9. A bomber below `air.bombers` [4], or below 4 × `eco.surplus_bomber_mult`
   [3] when surplus expansion is on and metal is full.

**Advanced air plant:** a gunship (ARMBRAWL / CORAPE) below `air.gunships`
[3].

**Shipyard.** The fleet target is the naval fleet target, times
`eco.surplus_fleet_mult` [3] when metal is full under surplus expansion.

- Submarine target = min(`navy.subs` [2], fleet target).
- Destroyer target = the rest of the fleet target.

First match:

1. A sea transport if the ferry is overdue.
2. A scout ship below 1.
3. A sea transport if the ferry priority is raised.
4. A construction ship below `navy.construction_ships` [2], only if the map
   has submerged metal.
5. A sea transport if wanted.
6. A destroyer below the destroyer target.
7. A submarine below the submarine target, once destroyers ≥
   `navy.subs_after_destroyers` [3].

With a fleet target of 0, the shipyard builds construction ships only.

**Advanced shipyard** (m = 3 with a full store under surplus expansion, else
1). First match:

1. An AA ship if air threat, below `navy.aa_ships` [2].
2. An advanced construction sub, once cruisers ≥ 2 and none exists, if a
   seaplane plant or underwater fusion is still wanted.
3. A cruiser if below `navy.cruisers` [4] × m and cruisers < 2 × (battleships
   + 1).
4. A battleship below `navy.battleships` [2] × m.
5. A cruiser below its target.

**Seaplane plant:**

1. A seaplane fighter if air threat and below the fighter target.
2. Otherwise a torpedo seaplane below `air.torpedo_planes` [6].

**Hover plant:**

1. Hover AA if air threat and below 2.
2. Otherwise hover tanks, with no ceiling.

**Air worth it** is recomputed every tick. It is true if air matters, or there
is an air threat, or ≥ `air.worth_it_defences` [4] remembered enemy armed
buildings exist.

**Island land-army cap** (`build.isolated_army_cap` [0, off]). With
unreachable ground, water fraction ≥ 0.8 and army ≥ the cap, land factories
queue only constructors, scouts and AA.

### 4.5 Expansion to metal spots

**Metal deposits.** Cells whose metal is above the map's surface metal are
*patch cells*. Patch cells touching at an edge or a corner form one *deposit*.

**Extractor site choice.** Search outward from an anchor, with candidate
cells ordered by ring, then row, then column. For each deposit, nearest first:

- consider footprint placements centred on its cells, up to (entry ring +
  footprint size) rings;
- reject placements that are off the map, that overlap a factory's clearance
  lane, that fail the placement test, or that cover no metal;
- score each placement: most metal under the footprint, then most centred on
  the deposit's centre of mass, then nearest. Ties are broken by the AI's
  random stream (§7).

The **heart wait**: if the best placement that ignores units would yield more
than the best free one (a unit is standing on the heart), the deposit waits up
to 60 s for it to clear, then takes the best free placement. The reasons, as
measured: a 4 × 4 deposit taken from the side yields 800 against 1800 from its
heart. A deposit is admitted if any one of its cells passes the ground rule.

**Search order** for an extractor job:

1. **Near search** from the builder, within `build.near_mex_radius` [1200].
   A site is refused if it is:
   - under guns: a remembered armed ground enemy within
     `build.mex_avoid_guns_radius` [450];
   - claimed: another builder has an order within `build.claim_radius` [96];
   - within 64 of a pending upgrade;
   - failed lately;
   - unreachable;
   - outside the commander's leash.
2. **Expansion search** from home, only for builders at base, with radius
   `build.commander_mex_radius` [1600] for the commander and
   `build.expand_radius` [4096] for others. It adds:
   - **our side** (`build.expand_our_side` [on]): the site is no closer to the
     enemy base than to ours. It always passes before the enemy base is
     known.
   - **explored** (`build.expand_explored_only` [off]).
   If only the our-side rule refused patches, the search is retried without it
   (`build.expand_middle_if_boxed` [on]).
3. **Wet preference.** If a submerged patch is nearer than the dry site found,
   drop the dry site, so the underwater extractor later in the list takes the
   wet one.
4. **Upgrade fallback** (`eco.upgrade_mex` [on]). This applies when there are
   no free patches, the builder is at base, no upgrade is pending, energy is
   not stalled, stock ≥ 0.5 × the tier-two extractor's cost, and no tier-two
   extractor frame exists. The AI then:
   - picks the nearest own extractor that is not under guns, is reachable, and
     has no other building within 72;
   - marks its removal as deliberate, so it is not treated as a loss;
   - reclaims it;
   - builds the tier-two extractor on the spot afterwards;
   - abandons the upgrade after `eco.upgrade_timeout_s` [180].

**Commander leash.** The commander's extractor searches are limited to 1600
from home, and to 1200 while any constructor exists
(`build.commander_near_sites` [on]).

**Free deposits on our side** drive the expansion constructor count (§4.4)
and rule E. A deposit counts as free when all of these hold:

- its centre is within 4096 of home;
- it is on our side, when the enemy base is known;
- it has no extractor within 48, whether ours, our frame, or a remembered
  enemy one;
- it is not under enemy guns.

### 4.6 Generic siting

**Clearance lanes.** Every building is surrounded by a lane: 2 tiles (32
units) normally, 3 tiles (48) round factories. A candidate whose expanded
footprint overlaps a factory lane is discarded. One that overlaps another lane
is kept only as a *crowded* fallback, used if no clean site exists.

A candidate is also rejected if:

- it buries a geothermal vent (unless it is a geothermal plant);
- any metal patch cell lies under its footprint;
- it fails the placement test.

**Ring search.** Candidate points lie on a grid with pitch = (larger footprint
side + 2) tiles, taken in square rings round the anchor.

- **Plain search.** First the nearest ring with any good site within
  `build.site_radius` [512], widening to `build.site_fallback_radius` [2048]
  if nothing is found. Then a random pick within that ring.
- **Scored search.** All rings out to the same radius; the highest score
  wins, and exact ties are broken at random.

**Under enemy guns.** This test reads remembered contacts only, needs
`build.notice_harassment`, and never counts aircraft.

- A remembered armed enemy *building* threatens its own weapon range + 32
  (`build.enemy_gun_range_from_weapon` [on], `build.enemy_gun_margin` [32]).
- Any other armed ground contact threatens 400.

**Site rules by type**, first match:

| Type | Site |
|---|---|
| Extractor, tier-two extractor, underwater extractor | §4.5 |
| Rebuild type | the lost tower's exact position (§4.8) |
| Fortification piece | its plan's slot (§4.8) |
| Light tower with an outpost plan, when the outpost was raided or the base already has its towers | plain search round the outpost anchor |
| Light or AA tower, at base (`def.spread` [on]) | the defence site (below) |
| Radar, at base | post = home + threat direction × `def.radar_distance` [320]; scored nearest ring, then highest ground |
| Light tower (fallback) | home + `def.distance` [160] towards the enemy base, or towards the strongest threat sector |
| Geothermal | the nearest explored, free, reachable vent |
| Torpedo launcher | 200 out from the first yard without a launcher within 400, towards the nearest remembered armed enemy unit, else the enemy base, else away from home |
| Water structures | ring search with no reachability test |
| Shipyard, advanced shipyard, seaplane plant | the shipyard site (below) |
| Solar at base (`eco.energy_rows` [on]) | energy rows (below) |
| Anything else | ring search round the anchor, reachable |

**Defence site.** Sites must be ground-reachable from home. A tower's
**spread** is its distance to the nearest same-type tower, capped at its range.

AA towers are scored by:

1. how many uncovered own buildings the new tower would cover;
2. spread;
3. −distance from home.

The first AA tower goes in the base middle, and later ones fill the gaps.

Other towers are scored by:

1. spread;
2. min(forward distance along the facing, post distance);
3. −distance from home.

The towers form a spaced line across the front, as far forward as the post.
The **post** = home + facing × post distance:

- Post distance is 160. While fewer than `def.perimeter_count` [3] towers of
  the type exist (`def.perimeter_first` [on]), it is max(160, min(outermost
  own building's distance + `def.perimeter_margin` [160], 900)).
- Facing (`def.face_losses` [on]) is the age-weighted mean direction of
  buildings lost in the last 60 s, with weight 1 − age/60 s. Without losses
  it is the direction to the enemy base, else to the map centre, else +x.
- If `def.face_threat_sector` [on] and sector memory has anything, the facing
  is overridden by the strongest sector's centre direction (§5.4).

**Energy rows.** A scored search round the anchor, with candidates on failed
cells excluded. Scores, in order:

1. adjacency: 1 if exactly one pitch from a same-type unit along a row or
   column, 0.5 if diagonal only;
2. −ring + (1 − cos(angle to the threat direction)) / 4, so within a ring the
   site away from the enemy wins;
3. −distance to the builder / 10000.

Solars form tidy rows behind the base.

**Shipyard site.** Taken from the precomputed sites, skipping:

- failed sites;
- sites within `navy.yard_spacing` [448] of our yards;
- illegal sites.

The nearest wins, except that an open site is preferred if it is at most 256
further. Exact ties are broken at random.

### 4.7 Protecting expansions (outpost towers and constructors)

**Outpost tower plan.** This needs a light laser tower role (ARMLLT /
CORLLT), an extractor role, and `def.outpost_towers` [3] > 0.

- **Outpost towers** are our light towers more than 900 from home.
- **The cap** is min(max(3, `def.outpost_max` [8]), 3 + floor(metal income /
  `def.outpost_income_step` [8])). That is 3 below 8 metal/s, plus one per 8
  metal/s, up to 8. No plan once the cap is reached.
- **Uncovered extractors** are finished extractors more than 900 from home
  and outside every light tower's range (about 300).
- **Raid sites** are recently lost extractors, plus remembered raided cells,
  that are more than 900 away and uncovered. They count even with nothing
  standing there, because that is where the rebuild goes.
- **Seed choice.** The candidates are each uncovered extractor and each raid
  site. A seed's score is (uncovered extractors within tower range of it) +
  (`def.outpost_min_mex` [2] if a raid site is within range). The best seed
  wins, with ties going to the earlier seed. A plan needs a score ≥ 2.
  Therefore two uncovered extractors that one tower could cover qualify, and
  so does a single raided site.
- **The anchor** is the mean of the covered extractors, or the raid site.
- The tower is built through rule 16 of §4.3. Its value test credits the
  covered extractors (45 s of income each). When the outpost was raided it is
  urgent: there is no value test, and it is allowed even when metal is short.
- With `def.outpost_towers_to_constructors` [on], the commander leaves outpost
  towers to any constructor able to build them.

**Extra constructors for expansions.** Each 4 free deposits on our side add
one constructor to the lab's target, up to 2 extra (§4.4). Rule E sends every
constructor beyond the first to expand.

**Guards for far build sites** (Hard and Brutal): see §5.6.

**Raids on outposts** are answered by the army (§5.4).

### 4.8 Defences: rebuild and fortification

**Rebuilding lost towers** (`def.rebuild` [on]):

- **Record.** Each lost armed building is recorded. Losses within 32 of an
  existing record merge into it and increment its loss count.
- **Forget.** Records are forgotten after `def.rebuild_memory_s` [1200], or on
  the loss after `def.rebuild_max` [3] rebuilds.
- **Wait.** After `def.rebuild_delay_s` [20], a record is eligible.
- **Skip** it if something of ours stands within 32 or is already ordered
  there, if it is under guns or failed, or if it is off the map.
- **Rebuild.** Rebuild the same type at the same spot. If a wreck blocks the
  spot, reclaim the wreck first.

**Attack watch** (every 15 ticks, needs `fort.where_attacked` [on] and a wall
role, ARMDRAG / CORDRAG):

- **Hits.** When one of our armed buildings loses health, the attackers are
  taken to be the remembered armed ground mobile enemies within
  `fort.attacker_radius` [500]. Their summed unit direction is the hit's
  direction.
- **Episodes.** Hits less than `fort.attack_gap_s` [20] apart form one
  episode. At most 6 episodes are kept per tower, each forgotten after
  `fort.attack_memory_s` [600].
- **Walls owed.** A tower is owed walls towards a direction when it has ≥
  `fort.repeat_attacks` [2] episodes, at least 2 of them within 45° of the
  latest one.

**Fortification plan.** There are three triggers:

1. **Every tower:** `fort.every_tower` [off], or `fort.at_tier_two` [on] once
   an advanced lab exists.
2. **Where attacked:** a tower is owed walls.
3. **Rebuilt:** `fort.rebuilt` [on] with rebuild records.

Wall geometry:

- **Wrap mode** (`fort.wrap` [on]). Wall-sized slots hug the tower's footprint
  at a gap of `fort.wrap_gap_tiles` [0], corners included: 8 slots for a 2 × 2
  tower and 2 × 2 walls. The slots facing the threat most squarely are used
  first.
- **Line mode.** A line across the facing, `fort.line_distance` [220] in
  front, filled middle first.
- **Walls wanted per side:** `fort.wrap_walls` [5] in wrap mode,
  `fort.reactive_walls` [3] in line mode, or `fort.walls_per_tower` [5] when
  walling every tower.
- **Slot validity.** A slot is skipped if a wall stands there, if it is failed
  or under guns, or if it is illegal.

Priority, one piece per call:

1. Towers owed walls on the attacked side.
2. Rebuilt sites: walls facing outward from home. At a site lost at least
   twice (`fort.aa_twice_lost` [on]), also an AA tower `fort.aa_back` [96]
   behind it, unless an AA tower is already within `fort.aa_cover_radius`
   [200].
3. Every finished light tower: walls facing the enemy base, then an AA tower
   behind (`fort.aa_tower` [on]).

### 4.9 Energy balance and metal makers

- **Energy.** Energy comes from the list itself:
  - solars to 4 in the opening;
  - an emergency solar whenever energy is stalled;
  - solars to 10, skipped while energy is rich;
  - tidal on naval maps;
  - geothermal on vents;
  - fusion at tier two when energy is short or metal is full.
- **Maker ceiling.** It is `eco.makers` [2]. It is raised to
  `eco.makers_starved` [6] once metal short and energy rich have both held
  for `eco.makers_starved_passes` [120] consecutive planning passes; the count
  resets the moment either fails. `eco.makers_surplus_max` [0, off] would
  allow more when energy income exceeds demand by more than 75.
- **Switching makers, every tick:**
  - All makers go **off** if an intruder is near base, if energy is stalled,
    or if energy is below `eco.maker_off_below_pct` [25]% of storage.
  - They go **on** above `eco.maker_on_above_pct` [70]%.
  - In between, they keep their state.
  - A maker is told at most once a second, and never while it is a frame.
- **Experimental budgeting** (`eco.maker_budget` [off]). This mode measured
  worse. Once a second, it ranks makers by metal per energy and switches on
  as many as the energy surplus pays for. One extra maker runs while energy is
  ≥ 95% of storage.

### 4.10 Constructor counts summary

| Builder | Count |
|---|---|
| Commander | 1. It builds the opening, then is round-robined like any builder. It has the extra rules in §5.7. |
| Level-one constructor | `build.constructors` [1], + up to 2 for free deposits, + 2 while capacity is short, + 1 while walls are owed |
| Advanced constructor | `build.adv_constructors` [1] |
| Air constructor | 1 |
| Construction ship | 2, on maps with submerged metal |

### 4.11 Repair and mending

- **Structures** (`build.repair` [on]).
  - **Which:** a builder repairs the nearest own defence, else factory, below
    `build.repair_below_pct` [90]% health within `build.repair_radius`
    [800].
  - **Skipped:** structures already held by `build.repairers` [2] repairers,
    unreachable ones, and ones under guns (unless `build.repair_under_fire`
    [off]).
- **Commander** (`build.repair_commander` [on], checked every 15 ticks). Below
  `build.repair_commander_below_pct` [90]% health, the nearest eligible
  builder within `build.repair_commander_radius` [1200] gets an immediate
  repair order, one per check, until `build.commander_repairers` [2] are on
  it. Nobody is sent if the commander's position is *exposed* (§5.7).
- **Mobile units** (`build.mend_units` [on]). One builder at a time heals the
  most damaged own mobile unit below `build.mend_below_pct` [50]% within
  `build.mend_radius` [700] of home. The commander does this only with
  `build.commander_mends` [off].

### 4.12 Reclaim

- **Harvest.** Idle fallback (§4.2), radius 1200 round home.
- **Field reclaim.** This applies while a wave of at least
  `reclaim.field_escort` [4] exists.
  - **Count:** reclaimable metal wrecks within `reclaim.field_radius` [700]
    of the wave centre, on explored ground and not under guns.
  - **Send:** one non-commander builder at base, at ≥ 4 wrecks, with up to
    `reclaim.field_batch` [6] reclaims queued, nearest first.
  - **Recall:** the builder goes home when fewer than 2 remain or the wave
    ends.
  - Only one field worker at a time.
- **Own wreckage.**
  - **Ripe:** a wreck within 32 of an own death site, at least
    `reclaim.own_delay_s` [10] old and not under guns.
  - **When:** only with no enemy near base (and only while metal is short, if
    `reclaim.own_only_when_short` [off]).
  - **Who:** up to `reclaim.own_reclaimers` [1] builders at once, each taking
    up to `reclaim.own_batch` [4] unclaimed ripe wrecks nearest to it.
  - **Pruning:** a site with no wreck is dropped after 5 s.

### 4.13 The timed opening (off by default)

`open.timed` [off] is a bias, not a script. It adds these rules:

- the lab from `open.factory_by_s` [60];
- a constructor by `open.constructor_by_s` [150];
- expansion extractors from `open.expand_by_s` [213], until
  `open.expand_sites` [10] outlying economy sites exist;
- the first wave at `open.first_wave_size` [7] from `open.first_wave_by_s`
  [450], unless the odds are known to be against us.

The timings were mined from human replays. Against the opponent it produced
identical games, so it stays off.

---

## 5. Army

### 5.1 Phases (the strategic state machine)

The phases are **Setup**, **Growth**, **Assault** and **Defend**. The machine
is evaluated every tick.

1. **Defend override, checked first.** If any intruder exists (§5.4), go to
   Defend, unless a wave is out. A wave is out when waves are on
   (`army.waves` [on]), `army.reserve_defends` [on] is set, the phase is
   Assault, and the wave is not spent. Even with a wave out, go to Defend if
   the sector threat total > `army.recall_ratio` [1.5] × the wave's metal
   (needs `army.proportional` and `army.sectors`). Entering Defend dissolves
   the wave.
2. **Setup → Growth** once any factory stands.
3. **Growth → Assault** needs a known target (an enemy base position or any
   contact) and any one of:
   - army ≥ `army.wave_size` [8];
   - hulls (less the naval scout) ≥ `navy.attack_size` [0, off];
   - **patience:** `army.patience_s` [240] since the last tick spent in
     Assault (the clock starts at the first Growth tick), with army ≥
     `army.spent_size` [3];
   - **odds:** the odds gate is open (below);
   - the timed opening's first-wave rule.
4. **Assault → Growth** when the wave is spent, or (without waves) when army <
   spent size. This also shuts the odds gate.
5. **Defend → Growth** once there are no intruders. It returns to Setup
   instead if no factory stands.

**Odds gate** (`army.odds` [on]):

- Shut while the known enemy army metal is 0.
- Opens when army ≥ max(spent size, ceil(wave size / 2)) and our army metal ≥
  `army.odds_open` [1.1] × the known enemy mobile ground army metal.
- Stays open until ours < `army.odds_close` [0.7] × theirs.

Our army metal counts combat units, excluding the borrowed scout.

### 5.2 Waves, reserve and objective

**Rally point.** The rally point is home + `army.rally` [220] towards the
enemy base (+x if unknown), or the ferry muster point when the army needs a
sea ferry (§5.9). Idle combat units more than 96 from it walk to it.

**Wave formation**, on entering Assault, when the wave is empty and not
spent:

- **Pool:** the unassigned combat units, in id order. Unassigned means not
  the scout, raid party, guard group, or a ferry passenger.
- **Split** (`army.reserve` [on]): the first round(n × `army.commit_frac`
  [0.7]) units, at least 1, form the wave, and the rest form the **reserve**.
  The health of each wave member is recorded.

**Reinforcement.** While the wave is out, whenever at least
`army.reinforce` [3] unassigned units exist, all of them join the wave.

**Contact** is either of:

- a wave member lost health since the last pass;
- a standing contact is within `army.meet_radius` [900] of the wave centre.

The **reserve joins** after `army.reserve_release_s` [20] of contact, or at
once if the wave is down to ≤ 3.

**Spent.** When the wave is < spent size [3], it is spent and the reserve
returns to duty.

**Objective**, recomputed every pass, first available:

1. The enemy commander's remembered position, if the hunt is allowed (§5.5).
2. The influence grid's best attack cell near the enemy base (§3.3).
3. The enemy base position.
4. Any remembered contact.

A candidate is skipped if its 192-cell has a recent failed ferry landing; if
every candidate is skipped, the first is used anyway.

**Meeting engagement.** If ≥ `army.meet_count` [3] remembered armed mobile
ground contacts are within 900 of the wave centre, the wave goes to their
centroid instead, so that two armies meet rather than pass.

**Movement.**

- Wave members with no target in reach attack-move to the objective
  (`army.attack_move` [on]).
- Cohesion (`army.cohesion` [0, off]) would make leaders stand still until the
  pack closed up. Walking leaders back oscillated and lost.

**Reserve and gatherers.** They hold at the rally point, except for answering
intruders (§5.4).

### 5.3 Measuring the enemy mix and choosing counters

**Role of a contact**, first match:

| Role | Rule |
|---|---|
| commander/other | a commander; an unarmed building; an unarmed mobile unit |
| builder | the unit can build |
| air | the unit flies |
| static defence | an armed building |
| tank | an armed mobile unit costing ≥ 200 metal |
| rocket/artillery | an armed mobile unit with longest range ≥ 300 |
| raider | any other armed mobile unit |

**Role histogram** (`mix.memory` [on]):

- Each tick, each role's count is raised to the number of contacts in that
  role remembered now. It is never lowered by that step.
- Every `mix.memory_halflife_s` [30], all counts are halved, using integer
  shifts.
- The histogram therefore follows the current army, not the lifetime total.

**Lean** (`mix.counter` [on]):

- Recomputed at most every `mix.commit_s` [60] and held in between, so a
  factory finishes its batch.
- A role **triggers** when its count is > 0 and its share of the histogram ≥
  `mix.counter_trigger` [0.3].
- Each triggered answer gets exactly `mix.lean` [2] share points. The value is
  either 0 or 2; it does not accumulate.

| Enemy role triggering | Kbot lab answer | Vehicle plant answer |
|---|---|---|
| air or raider | rocket kbot | missile truck |
| static defence or tank | artillery kbot | medium tank |
| rocket/artillery or builder | raider kbot | light tank |

**Static/mobile rule** (kbot lab only, applied every pass). Over remembered
armed ground contacts, sum the metal of buildings and of mobile units.

- If buildings ≥ trigger × total, artillery gets +`mix.counter_bonus` [2].
- If mobile ≥ trigger × total, rocket kbots get +2.
- Both rules can fire at once.

The most a share can rise is therefore +4.

**Faction defaults** (§6.3) set the base shares. ARM's lab, for example, has
all-zero base shares, so it builds only what the lean asks for.

### 5.4 Defence: intruders, sectors, proportional response

**Intruders** are remembered armed contacts, aircraft included, within
`army.defend_radius` [900] of home, or within 400 of a besieged factory.

**The target intruder** is the intruder nearest home that is seen now,
standing and not a frame. The metal and count of all such intruders are
summed.

**Holding** (`army.hold_outnumbered` [on]). If intruders outnumber our combat
units, nobody charges. Units still fire at what enters their reach, and
otherwise hold the rally point or the radar line.

**Proportional response** (`army.proportional` [on]).

- **Pool:** unassigned units, excluding the wave while in Assault. The pool
  is sorted by distance to the target intruder, then by id.
- **Bounds:**
  - minimum = clamp(`army.respond_min` [1], 1, pool);
  - maximum = ceil(`army.respond_max_share` [1.0] × pool);
  - count floor = max(minimum, min(intruder count, maximum)).
- **Picking:** take units in pool order until the maximum is reached, or until
  both count ≥ floor and metal sent ≥ `army.respond_ratio` [1.3] × the
  intruder metal.
- **Who acts:**
  - in Defend, the responders attack the target intruder;
  - in Assault, responders who are not in the wave attack it;
  - everyone else holds.

**Commander alone.** In Defend, with no combat units and at most
`cmd.defends_alone_max` [1] intruder, the commander attacks it if it can fire
from where it stands.

**Sector threat memory** (`army.sectors` [on]):

- **Geometry:** 16 fixed sectors of 22.5° round home. Sector 0 starts at
  bearing −180° and the index increases with angle.
- **Adding threat:** when a contact first becomes an intruder, its metal
  (rounded, at least 1) is added to its sector. It counts once while it stays
  in; one that leaves and comes back counts again.
- **Decay:** every `army.sector_halflife_s` [60], all sectors are halved.
- **Uses:**
  - the **total** drives the wave recall (§5.1);
  - the **strongest sector** (ties go to the lowest index) faces towers and
    the light-tower fallback post (§4.6).

**Radar early warning.**

- **Sampling:** once a second, every mobile non-own unit within
  `army.radar_rings` [2.5] × 900 = 2250 of home is examined. Seen units must
  be armed. Unseen ones must be on radar.
- **Closing:** a unit is closing if it is more than 8 nearer than at the last
  sample.
- **Warning:** with ≥ `army.radar_min_contacts` [2] closing units, the warning
  direction is their centroid, held for 10 s.
- **Response:** with no intruder present, units in the default branch go to a
  7-slot line at 540 from home across that bearing. The slot is (unit id mod
  7) − 3, slots are 56 apart, and a unit is left alone within 72 of its slot.

**Outpost raids** (`army.outpost_response` [on]), only with no intruder at
base.

- **Site:** the first own building more than 900 from home that has a seen,
  armed ground mobile enemy within `army.outpost_raid_radius` [350].
- **Responders:** all unassigned non-reserve units within
  `army.outpost_response_radius` [1500] of the site.
- **Decision:** they attack only if their metal ≥ `army.outpost_strength`
  [1.2] × the raiders' metal. The alternative estimate rule (§5.12) is off.
- One outpost per pass.

### 5.5 Hunting exposed commanders

When `army.hunt_commander` [on], the enemy commander's last known position
becomes the wave objective, if it is allowed. With `army.hunt_exposed_only`
[on] it is allowed only while the commander is **exposed**:

- its remembered position is ≥ `army.hunt_min_distance` [600] from its base
  centroid (when known); and
- the remembered armed non-commander contacts within
  `army.hunt_escort_radius` [450] of it are worth < `army.hunt_max_escort`
  [600] metal.

The verdict is held for `army.hunt_latch_s` [10], and cleared when the
commander's position becomes unknown.

At sea, while the fleet is out, submarines chase a known enemy commander on
the same water body. They attack if it is seen now, else move to its last
position.

### 5.6 Raids, guards and captures

**Raiding party** (`army.raids` [on]). Formed only in Assault, with a wave out
and no intruders. It is cleared when the wave is spent or empty.

- **Who:** `army.raid_size` [3] unassigned units in id order. With fewer
  available, there is no raid ("a raid of one is a gift").
- **Target:** a remembered enemy building that meets all of these:
  - standing;
  - owned by the focus enemy;
  - ≥ `army.raid_avoid_base` [900] from the enemy base;
  - on a cell with zero ground threat.
  The nearest to our home wins.
- **Fresh frames first** (`army.raid_frames` [on]). Prefer a target that was a
  frame when seen, was first seen within `army.raid_frame_window_s` [30], and
  is not now seen finished. If an eligible enemy builder is seen within 480
  of that frame, raid the builder instead. Raiders get an explicit attack
  order when the target is seen and hittable; otherwise they walk to it.
- **Sync** (`army.raid_sync` [on]). The party waits at the rally point until
  any of:
  - `army.raid_sync_s` [15] pass;
  - the wave makes contact;
  - a contact is within 900 of the wave centre;
  - the wave is within 900 of its objective.

  The party is then released once. This makes the raid a second axis timed
  with the main push.

**Build-site guard** (`build.site_guard_size`: 0 on Easy and Standard, 3 on
Hard and Brutal).

- **Request:** made when a builder is sent to a site at least
  `build.site_guard_min_distance` [900] from home, and the summed ground
  threat within `build.site_guard_threat_radius` [600] is ≥
  `build.site_guard_threat` (0 = always; 1000 on Hard and Brutal). A new
  request replaces the held one only if it is at least as threatened.
- **Guarding:** that many unassigned units stand within 128 of the site.
- **Release:** when the builder dies, when a building of ours stands within
  64, or after `build.site_guard_timeout_s` [180].

**Commander guard.** While the commander is in danger, army units and hulls
within `cmd.guard_radius` [1500] attack its nearest threat. If no threat is
known they move to it, when more than 160 away.

**Anti-air escorts.** In Assault, `def.aa_escorts` [1] of the lowest-id
mobile AA guard the slowest wave member. At least one AA always stays home.

**Capture.** The AI never captures, even though the commander can.

### 5.7 Builder safety and the commander

**Exposure.** A position is **exposed** when, counting only remembered contacts
seen within the last 150 ticks:

- **Threat metal:** sum the metal of armed non-air contacts whose reach covers
  the position. Reach is weapon range + `safety.margin` [150] for mobile
  units, or + 32 for buildings. Frames do not count.
- **Cover metal:** sum the metal of our armed finished non-air units that
  cover it. A mobile unit covers it if it is within `safety.cover_radius`
  [500] and not more than 64 behind the position relative to the threat. A
  building covers it if its range covers the threat centre (+32) or the
  position. The commander counts as `cmd.fight_max_metal` [600] metal of
  cover.
- **Exposed** when cover < threat × `safety.cover_ratio` [1.0].

**Builder retreat** (`safety.builders` [on], every 15 ticks):

- **Who:** every exposed mobile, non-commander, non-flying builder, unless its
  frame is ≥ `safety.finish_above_pct` [70]% done.
- **Where:** it is moved away from the threat centre by max(96, max threat
  range + 150 − current distance). If home lies in that direction, it heads
  for home, and goes straight there if home is close. The point is clamped 64
  inside the map.
- **After:** it is not re-tasked for `safety.shelter_s` [10]. With
  `safety.resume` [on], a "finish that frame" order is queued behind the move,
  so no duplicate frame is placed.

**Commander danger** (`cmd.danger_radius` [450]). The commander is in danger
for 10 s after it loses health, or while a seen, standing, armed, non-air,
non-frame contact is within 450. For a contact on water the radius is the
contact's own weapon range, if longer.

**Fight or flee.** The commander **fights** when all of these hold
(`cmd.stands_ground` [on]):

- it can fire from where it stands (above water, or it carries an underwater
  weapon);
- its health is ≥ `cmd.retreat_pct` [50]%;
- the threat metal is ≤ `cmd.fight_max_metal` [600];
- not every threat is a ship (`cmd.shuns_ships` [on]).

**When fighting:**

1. **Keeping a frame** (`cmd.keeps_frames` [on]). Applies when the commander
   has a frame in hand. A frame's remaining life is its built fraction × its
   energy cost, in ticks. If that life is shorter than
   `cmd.frame_absence_s` [20], and our armed units within
   `cmd.frame_cover_radius` [800] match the threat metal, the commander stays
   on the frame. Otherwise it may hand the frame to the nearest suitable
   builder within `cmd.frame_handover_radius` [800], unless that builder's
   position would be exposed.
2. **D-gun** (`cmd.dgun` [on]). The commander's special weapon (its third
   weapon) fires when stored energy covers a shot.
   - **Target:** seen, standing, armed, non-air contacts on dry ground within
     range, the most valuable first (`cmd.dgun_by_value` [on]), then the
     nearest.
   - **Friendly-fire check:** no shot if any of our own units is within 48 of
     the line of fire out to range + 32. Allies are not checked.
   - A repair of its old frame is queued behind the shot.
3. **Otherwise** it attacks the nearest land threat, with the same queued
   return to its frame.
4. **Chase leash.** A chase is called off beyond 675 (1.5 × 450).

**When fleeing:**

- **Ships:** choose a dry point out of every ship's reach + 100. The search
  covers 16 bearings at distances 200, 350, 500, 700, 900 and 1200, and every
  point must be dry along the way. The first ring with a qualifying point
  wins, taking that ring's roomiest point. Failing that, take the roomiest
  dry point anywhere, if it beats the current spot by 100. Failing that, go
  home if it is out of ship reach, else hold position.
- **Land threats:** go home if more than 300 from it. Otherwise move 350
  directly away from the nearest threat, and if that point is underwater, go
  home instead.

**Harassment** (`cmd.answers_harassment` [off]): with a besieged factory and
no army or navy, the idle commander attacks the nearest seen enemy within 450.

### 5.8 Air

The air pass owns armed aircraft. Each tactics pass:

- **Wait point:** a repair pad, else an air plant, else home. Idle aircraft
  further than `air.leash` [2400] from it return there. There is no explicit
  "go and repair" behaviour; aircraft are repaired by idling at the pad.
- **Standing patrol.**
  - **Who:** the first `air.patrol_fighters` [2] fighters and the first
    `air.patrol_bombers` [2] bombers, by id.
  - **Route:** 4 points at −60°, −20°, +20° and +60° from the enemy-facing
    direction (map centre if unknown). The radius is clamp(base edge, 200,
    900) + `air.patrol_margin` [400].
  - Patrol bombers never join sorties.
- **Fighters.** Each fighter attacks a seen enemy aircraft, armed ones first,
  then the nearest. There is no chase leash.
- **Bombers** (ready when ≥ 2 non-patrol bombers exist, so 4 bombers in all
  by default). First available target:
  1. **Home defence:** the nearest remembered armed ground contact within
     `air.bomb_home_radius` [900] of home. This ignores AA.
  2. **Building sortie:** remembered standing enemy buildings under AA cover
     ≤ `air.bomb_max_aa` [2]. Score:
     - metal;
     - × `air.bomb_factory_weight` [2] if it is a factory;
     - ÷ (1 + cover × `air.bomb_aa_penalty` [1]);
     - ÷ (1 + distance from home / `air.bomb_distance_scale` [2400]);
     - × `air.bomb_army_discount` [0.25] if within `air.bomb_army_reach`
       [700] of the wave, since the army will deal with it.
  3. **Cluster:** a seen mobile ground contact under cover ≤ 2 with at least
     `air.bomb_cluster_min` [3] standing ground contacts within
     `air.bomb_cluster_radius` [300]. The largest cluster wins; ties go to
     the one nearer home.
- **Gunships** (ready at ≥ `air.gunship_pack` [2]). Target the seen armed
  ground contact nearest the wave centre, within `air.gunship_radius` [1400]
  of it, under cover ≤ 2. Otherwise they take the bomber target.
- **Joint ground and air timing** (`air.sync` [on], `air.sync_radius` [600]).
  Bomber and gunship strikes are held while all of these are true:
  - the phase is Assault and the wave is out, not spent, and has not made
    contact;
  - the target is not within 900 of home;
  - the wave centre is not within 600 of either its objective or the air
    target.

  As soon as any of these fails, the strike is released. Home defence and
  fighters are never held.
- **Torpedo seaplanes** (at ≥ 2). Seen contacts over water, mobile ones first,
  then the nearest to home.

### 5.9 Transports and ferries on water maps

**When a ferry is wanted.**

- **Enemy across water:** there is an objective and it is not
  ground-reachable. Once observed, the answer is *latched*, because a hull
  takes about 220 s to build.
- **Army needs a ferry** (`ferry.want_from_map` [on]): the latched answer, or,
  before any objective exists, "no land route to any other start position".
- **Expansion wanted:** an extractor site exists on walkable ground that home
  cannot reach. This is checked at most every 10 s, with no radius limit.
- **Transport wanted** = army needs a ferry, or expansion wanted.

**Ferry urgency** (`ferry.urgent` [on]). Applies when unreachable ground
exists, the enemy base is known, there is no land route, and we own no ferry.
Where a shipyard and a sea transport role exist, only a completed sea
transport counts as owning a ferry.

- **Raised** before `ferry.first_by_s` [480]: the sea transport goes ahead of
  the construction ship and the warships.
- **Overdue** after that: it goes ahead of the scout ship too.

A sea lift is preferred over an air lift when a shipyard can build one. This
rule turned 3 of 4 water-map games into faster wins and regressed one into a
timeout.

**Builder ferry (islands).**

- An idle transport takes one idle non-commander ground builder from home
  ground to the island extractor site.
- There is no landing search and no threat check.
- This takes priority over army ferries.

**Army ferry.** Only in Assault, with the enemy across water now and a home
point known. For each transport with no trip and nothing aboard:

- **Capacity:** air transports carry 1, whatever their data says; sea
  transports carry their real capacity.
- **Minimum party:** a sea transport waits until total combat units ≥
  `ferry.min_party` [6]. Air transports, and hulls smaller than the minimum,
  are exempt.
- **Passengers** are combat units on home ground that:
  - are not booked;
  - are not hovercraft (they cross water themselves);
  - fit the transport's size limit;
  - do not need water.
- **Landing search.** Rings of 48 units from 192 out to 48 ×
  `ferry.landing_rings` [24] from the objective, on bearings within ±90° of
  the homeward direction (`ferry.landing_fan` [on]): 9 bearings, nearest
  home first. A candidate must be dry, ground-walkable and not reachable from
  home. A sea landing must also have naval-reachable water within 150 towards
  home, which stays inside the 200 unload reach. With
  `ferry.landing_avoid_threat` [on], each ring is scored by anti-air cover
  (air carriers), then ground threat within `ferry.landing_threat_radius`
  [300], then distance from the origin. The search stops at the first ring
  whose best candidate has zero threat. As a fallback, the objective itself
  is used if it qualifies.
- **Refusal memory.** A failed search remembers the objective's 192-unit cell
  for `ferry.refusal_s` [30]. A success clears the cell.
- **Muster** (`ferry.muster` [on]). The sea muster is found by searching rings
  of 48 out to 1440 from the shipyard (else home), on 16 bearings, for a dry
  reachable shore point with naval-reachable water within 144. While the army
  needs a sea ferry and we own no air transport, the rally point moves to
  this shore.
- **Orders.**
  - **Mustered sea ferry:** passengers move to the shore; the hull moves to
    the water point. Once the hull is within 64 of it, it is given the loads
    (the first Immediate, the rest Queued) and then **one** queued unload.
    Never queue identical unloads: the game toggles duplicates off.
  - **Other ferries:** the transport loads each passenger where it stands,
    then unloads.
  - **Unloading:** once the queue is empty while still carrying, issue one
    more unload. Each unload sets down one unit.
- **Trip upkeep.**
  - A trip ends when the transport is loaded and then empty.
  - It is called off (Stop) if it has no living passengers, or if the pickup
    has not happened within 120 s.
  - A loaded sea hull that makes no progress (moves less than 24, carry count
    unchanged) for `ferry.stall_s` [20] refuses its landing cell and
    searches again from where it is.
- **Missing:** escorts, return trips and reinforcement ferries do not exist,
  and air ferries are one unit per trip.

**Hovercraft** (where the data has them) cross water by themselves. A hover
plant is built only when `build.hover_plants` > 0 (off by default), the army
needs a ferry, a hover route exists, and income ≥ 10.

### 5.10 Scouting

- **Scouts.** Scouts are scout units. If there are none, none are being built,
  and none are owned, the first combat unit is borrowed (and not counted in
  the army) until a real scout appears. On water maps with nothing enemy
  known, one hull is borrowed (`scout.naval` [on]); it returns once any enemy
  is known.
- **Each scout pass** (`scout.enabled` [1]), a scout with orders is left
  alone. Otherwise:
  - **Hurt** (below 1/3 health): it goes home if more than 256 away, and gets
    nothing else.
  - **Opening:** while the enemy base is unknown, the first leg is the
    stalest declared start position more than 512 from home whose cell has
    staleness > 20.
  - **Other legs:** planes get 3 legs in all, ground and sea scouts 1. Each
    leg maximises min(staleness, 3000) / distance from the previous leg.
  - **Accept test:** ≥ 320 from other scouts' claims and from earlier legs;
    zero ground threat in the cell (planes too); and the right terrain and
    reachability for the scout.

### 5.11 Damaged units

`army.retreat_damaged` [on]:

- **Retreat threshold:** below `army.retreat_raider_pct` [30]% health for the
  side's raider kbot, else below `army.retreat_line_pct` [45]%.
- **Conditions:** a builder exists (`army.mend_needs_mender` [on]); the unit
  is within `army.mend_max_walk` [1800] of home; and nothing is attacking the
  base.
- **Where:** it walks to one of 8 mend spots `army.mend_spot_radius` [160]
  round home, chosen by a hash of its id. It moves only if further than
  `army.mend_spot_tolerance` [400] from the spot.
- **Back to duty:** it rejoins at `army.rejoin_pct` [60]%, or after
  `army.mend_wait_s` [45], whichever comes first. It is not sent home again
  until healed.

### 5.12 Per-unit micro

For each combat unit, first match:

1. Damaged retreat.
2. A target in reach.
3. Commander guard.
4. Outpost response.
5. Build-site guard.
6. Phase duty: Defend responder; raider errand; wave objective; Assault
   responder; reserve at rally; radar line; gather at rally.

**Acquisition.**

- **Radius:** max(`army.engage` [450], longest weapon range −
  `army.kite_margin` [40]) (`army.acquire_at_range` [on]).
- **Target:** the nearest seen, standing, non-frame contact in that radius
  that the unit has not given up on.
- **Frames:** never targeted, except by raids. A frame on a factory pad would
  otherwise shield the factory.
- **Known flaw:** the land search does not check that the unit's weapons can
  hit the target, so it may pick an aircraft it cannot shoot.

**Blocked shots** (`army.blocked_shots` [on]):

- **When:** the shooter has a direct-fire weapon (not lobbed), neither side is
  an aircraft, and the target is within the shooter's range.
- **Test:** sample the line between the two muzzle points, 24 above the
  ground, about once per tile (2-24 samples). If the ground rises more than
  12 above the line, the shot is blocked.
- **Fix:** try 8 points along the approach, 1/9 to 8/9 of the way, and move to
  the first with a clear line. If none is clear, walk to the target.

**Stalled attacks** (`army.stalls` [on]):

- **Clock:** per (unit, target) pair. It runs only while the target is within
  the unit's reach, and restarts whenever the target's health changes from
  anyone's fire.
- **Repositioning:** after `army.stall_s` [15] without damage, the unit
  repositions, up to `army.stall_tries` [2] times. It moves to a clear-shot
  point if terrain is the problem; otherwise it sidesteps
  `army.stall_sidestep` [240] across the line of fire plus 120 forward,
  alternating sides.
- **Giving up:** after that, it ignores the target for `army.stall_forget_s`
  [60] and stops.

**Kiting** (`army.kite` [on]). Against a mobile target whose longest range +
40 is less than ours, if we are closer than (their range + 40), step straight
back to that distance. Buildings are never kited.

**Focus fire** (`army.focus_fire` [off]). This rule measured as a regression
against the opponent. The wave would share one target, held for
`army.focus_latch_s` [5], picked in tiers:

1. armed buildings and long-range units;
2. builders and commanders away from their base;
3. other armed units;
4. the rest.

Within a tier, higher metal first, then nearer.

**Engagement estimate** (used only by the off-by-default outpost option). A
side's score is the sum over its units of health × best-weapon dps × (1 +
range bonus). The range bonus is clamp(((range − opposing mean range) /
max(0.05, own speed + opposing mean speed)) / 45, 0, 3), and is 0 if the unit
does not outrange the opposing mean. The margin is score A − score B.

**Navy.**

- **Station.** Hulls gather at a station `navy.rally` [320] out from the first
  shipyard, on the first of 16 bearings with deep, connected water (at least
  20 deep at the station, at least 12 along the way).
- **Sailing.** The fleet sails when ≥ `navy.sail_size` [3] hulls are within
  440 of the yard. It is recalled below 2.
- **Objective:** the remembered enemy on our water body nearest the enemy
  base; else the shipyard site nearest the enemy base; else start positions
  more than 900 from home, furthest first, rotating every 120 s.
- **Per hull:**
  - commander guard within 1500 on the same water;
  - submarines hunt the commander;
  - attack seen naval-reachable enemies within 450, standing off when the
    hull outranges them (`navy.stand_off` [on]), buildings included. The
    stand-off distance is max(their range + 40, our range − 40);
  - stall repositioning after `navy.stall_s` [12]: 8 bearings round the
    target, at radii 240, 340 and 160.

### 5.13 Base-loss recovery

The AI has no dedicated recovery mode. Recovery comes from the general rules:

- Defend falls back to Setup when no factory stands.
- The priority list's "no lab" rule (rule 7) is near the top, so the next
  idle builder rebuilds the lab.
- Lost buildings are promoted to the front of the list for 60 s.
- Lost towers are rebuilt in place after 20 s.
- With no home point there is no rally, no defence line, no mending and no
  outpost response.
- With no shipyard the navy freezes.
- If the commander dies, the home point keeps its old value. The game usually
  ends anyway when the commander dies.

---

## 6. Personalities and difficulty

### 6.1 How a seat's tuning is assembled

The layers are applied in this order; later layers overwrite earlier ones.

1. **Difficulty profile** (§6.2). The difficulty is the personality's own, if
   it names one, else the game's.
2. **Faction defaults** (§6.3).
3. **Personality knobs** (§6.4), in file order.
4. **Arena overrides** (§8), in command-line order. An unknown knob is fatal
   here.

### 6.2 Difficulty tiers

Difficulty changes counts and cadence, and Brutal also cheats. Knobs not
listed keep the Standard default.

| Knob | Easy | Standard | Hard | Brutal |
|---|---|---|---|---|
| `eco.solar` | 6 | 10 | 12 | 10 |
| `eco.mex` | 5 | 8 | 10 | 8 |
| `build.constructors` | 1 | 1 | 2 | 3 |
| `eco.extra_labs` | 0 | 1 | 2 | 1 |
| `build.vehicle_plants` | 0 | 1 | 1 | 1 |
| `def.towers` | 1 | 2 | 3 | 3 |
| `def.base_aa_towers` | 0 | 1 | 2 | 2 |
| `def.reactive_aa_towers` | 1 | 3 | 5 | 5 |
| `def.mobile_aa` | 0 | 2 | 3 | 3 |
| `army.wave_size` | 14 | 8 | 7 | 6 |
| `army.spent_size` | 5 | 3 | 2 | 3 |
| `cadence.plan_ticks` | 60 | 30 | 20 | 20 |
| `cadence.tactics_ticks` | 30 | 15 | 10 | 10 |
| `cadence.scout_ticks` | 120 | 60 | 45 | 60 |
| `def.face_losses` | off | on | on | on |
| `def.payback_s` | 0 (no test) | 90 | 60 | 45 |
| `def.payback_per_mex_s` | 45 | 45 | 30 | 20 |
| `build.site_guard_size` | 0 | 0 | 3 | 3 |
| `build.site_guard_threat` | 0 | 0 | 1000 | 1000 |
| `navy.fleet` | 3 | 6 | 9 | 12 |
| `navy.subs` | 0 | 2 | 3 | 4 |
| `navy.subs_after_destroyers` | 3 | 3 | 2 | 2 |
| `reclaim.own_reclaimers` | 1 | 1 | 2 | 2 |
| `reclaim.own_delay_s` | 30 | 10 | 0 | 0 |
| `reclaim.own_only_when_short` | on | off | off | off |
| `cheat.full_vision` | off | off | off | **on** |
| `cheat.income_factor` | 1 | 1 | 1 | **1.25** |
| Micro limit (orders/min, §2.3) | 60 | 120 | none | none |

**Idle** difficulty constructs the seat (it still draws its random seed, §7)
but never issues anything.

The income factor multiplies the player's metal and energy income in the
simulation's economy step. The host must apply it: the AI only declares it.

### 6.3 Faction defaults

These are applied after difficulty.

| Knob | ARM | CORE |
|---|---|---|
| `mix.lab_raider` | 0 | 0 |
| `mix.lab_rocket` | 0 | 4 |
| `mix.lab_artillery` | (0) | 2 |
| `mix.vp_light_tank` | (1) | 3 |
| `mix.vp_missile` | (0) | 1 |
| `mix.vp_medium_tank` | (0) | 2 |
| `mix.counter_bonus` | (2) | 2 |
| `mix.counter_trigger` | (0.3) | 0.3606 |
| `army.wave_size` | (tier) | tier + 6 (Easy 20, Standard 14, Hard 13, Brutal 12) |

Values in brackets are unchanged from the defaults.

Why:

- **ARM's kbot lab** builds nothing on its own account. Its raiding is done by
  light tanks from the vehicle plant, and the lab answers only the counter
  lean. Measured: better in 25 of 32 paired games, p < 0.001.
- **CORE** does not contest the raider fight. It relies on rocket and
  artillery kbots that outrange, plus tanks.

Any side that is not CORE is treated as ARM for the role table.

### 6.4 Built-in personalities

| Knob | Balanced | Rush | Turtle | Tech Rush |
|---|---|---|---|---|
| `open.mex` | (3) | 2 | | |
| `open.solar` | (4) | 2 | | |
| `def.towers` | (tier) | 0 | 6 | 1 |
| `def.heavy_towers` | (2) | 0 | 4 | |
| `def.outpost_towers` | (3) | 1 | 5 | |
| `def.outpost_income_step` | (8) | 0 | 5 | |
| `def.outpost_max` | (8) | | 12 | |
| `def.outpost_min_mex` | (2) | | 1 | |
| `fort.every_tower` | (off) | | on | |
| `fort.where_attacked` | (on) | off | | |
| `def.base_aa_towers` | (tier) | | 2 | |
| `army.wave_size` | (tier/faction) | 5 | 20 | 12 |
| `army.spent_size` | (tier) | 2 | 6 | |
| `army.reinforce` | (3) | 2 | | |
| `mix.lab_raider` | (faction) | 4 | | 1 |
| `mix.lab_rocket` | (faction) | 1 | | |
| `mix.lab_artillery` | (faction) | 0 | | |
| `army.raids` | (on) | on | off | |
| `army.raid_size` | (3) | 3 | | |
| `tech.enabled` | (on) | off | | on |
| `tech.min_income` | (10) | | | 6 |
| `tech.min_value_ratio` | (1.5) | | | 0 |
| `eco.save_tech_s` | (240) | | | 480 |
| `build.adv_constructors` | (1) | | | 2 |

Notes on the table:

- Balanced is the tuned default with no knobs of its own.
- A blank cell means the personality does not set that knob.
- A personality's knobs overwrite the faction defaults. Rush on CORE
  therefore builds raider kbots.

There are also built-ins that set only a difficulty: **Easy**, **Medium**
(Standard) and **Hard**. Brutal and Idle can be selected through a
personality file or the host's difficulty setting.

### 6.5 Personality files (format, as data)

A personality is a UTF-8 text file with the extension `.txt`. Personality
files are kept in a dedicated `ai` folder in the user's local data directory.
(This is not the game archive's AI folder: the original game's AI profile
files are a different format and are not read by this AI.)

- **Lines.** One `key=value` per line. Whitespace round keys and values is
  trimmed. Blank lines and lines starting with `#` or `;` are ignored. Any
  other line without `=` is an error.
- **Reserved keys** (case-insensitive):
  - `name` (default: the file name without its extension);
  - `description`;
  - `difficulty`: `idle` / `none` / `off`, `easy`, `standard` / `medium` /
    `normal`, `hard` or `brutal`;
  - `engine`: which AI engine plays the seat. A file is this AI's unless it
    names another engine. A file for another engine may not set knobs.
- **Knobs.** Every other key is a knob name (Appendix A).
  - Booleans: `1`, `true`, `on` or `yes` are true; anything else is false.
  - Integers and numbers are parsed as such.
  - An unknown knob, or an unparseable value, rejects the whole file with a
    message naming the line. This is deliberate: a misspelt knob that did
    nothing would make a personality look as if it worked.
- **Loading.** Built-ins come first. Then every `*.txt` in the folder is read
  in file-name order. A file whose name matches an existing personality
  (case-insensitive) replaces it; otherwise it is added. A file that fails to
  parse is skipped and logged.
- **Lookup.** By name, case-insensitive.
- **Seats.** A personality is applied when the seat is created. Saves record
  the personality name. Replays do not need it, because they replay commands.

Example:

```
# A raider that never builds towers.
name=Raider
description=Raids from the first minute.
difficulty=standard
army.wave_size=4
def.towers=0
```

---

## 7. Determinism and lockstep

- **The AI is part of the simulation.** It runs on every peer, inside the
  simulation tick, after unit updates and before command application. Its
  orders are never sent over the network.
- **Fixed delay.** Its orders are delivered through the same command buffer as
  human orders, but at a **fixed** depth of 7 ticks on every peer. The delay
  must never depend on network round trip or frame rate. AI orders must be
  fed into the buffer once per *simulated* tick, never once per rendered
  frame or per attempt to tick. The reference AI desynced three times over
  exactly these points before this was fixed.
- **Random stream.**
  - **Seeding:** each computer seat, in seat order, draws exactly one value
    from the simulation's own random stream when it is created, idle seats
    included. That value seeds the seat's private generator. Start positions
    are dealt *after* these draws, so seat creation order is part of the game
    hash.
  - **Generator:** a minimal-standard linear congruential generator (Park-
    Miller, multiplier 48271, modulus 2³¹ − 1). Draws are `raw mod n`, never a
    library distribution, because those vary between standard libraries.
  - **Uses:** only to break exact ties in site choice (plain ring pick, scored
    site ties, extractor placement ties, shipyard ties, AA tower siting in
    fortification). Nothing else is random. The army, air and scout passes
    use no randomness: ties go to the lower unit id, and spreading uses id
    arithmetic.
- **Ordering.** Every iteration that can influence an order must be in a
  defined order: unit id, player id, feature id, or grid scan order. Never
  use hash-map order or pointer order.
- **Arithmetic.** Positions use the simulation's fixed-point scalar. Some
  scores use floats; keep them deterministic across peers (same compiler
  flags, no fused multiply-add differences, and a deterministic atan2 for the
  sector angle). The reference AI computes sectors with a deterministic
  arctangent.
- **No wall clock.** Profiling may time passes, but must never feed back into
  decisions.
- **State and saving.** The AI's state is *not* saved. On load the AI starts
  from nothing and re-plans. This loses memories such as contacts, latches,
  timers, failed sites and the role histogram, so a loaded game plays
  differently from the uninterrupted one (it stays in sync, because every
  peer loads the same way). The host's save interface lets an engine save
  state, but this AI does not use it.

---

## 8. The tuning arena

The arena exists to tune and verify the AI, and the implementer should build
an equivalent one.

### 8.1 One headless game

**Inputs:**

- map;
- game-time cap in seconds;
- seed;
- seats (name, side, colour, optional team; computer seats only);
- AI engine (classic opponent or this AI);
- difficulty;
- start mode (fixed or random);
- commander-death rule;
- per-seat personality;
- per-seat knob overrides (`seat:knob=value`);
- watchdog in wall seconds;
- output folder.

**What determines the game.** The simulation's random stream is seeded from
the seed, the map name, and each slot's colour, starting resources and name.
Then each computer seat draws its AI seed. Then random start positions are
dealt by a Fisher-Yates shuffle from the back. Same inputs, same game,
byte-identical.

**Loop.** Feed AI orders at the fixed depth, apply commands, tick the sim,
sample the report. The game ends one tick after the game reports a decision
(a win or a draw), or at the cap ("timeout"), or when the watchdog fires.

**Outputs:**

- **Per-player CSV** every 10 s, with a final row at the end:
  - stock, storage, income and demand of metal and energy;
  - unit, building, army, builder and factory counts, including idle ones;
  - losses;
  - army metal;
  - metal and energy produced and wasted;
  - phase.
- **Per-unit events**: first seen, completed and died, plus the death context
  within 600 (enemies near, nearest enemy type, friendly army and towers near,
  cause, killer).
- **Run summary**: build id, map, seed, duration, ended (decided or timeout),
  winner, ticks, and each seat's engine, difficulty, personality and
  overrides.
- **Event log**, byte-identical for the same seed.
- **A one-line result.**

### 8.2 Paired games and phases

All tuning games are this AI against itself: the candidate tuning against the
defaults.

**Scenarios:**

- ARM mirror;
- CORE mirror;
- ARM-vs-CORE cross.

In mirrors, the candidate takes seat 0 on odd seeds and seat 1 on even seeds.
In the cross, both seats get the candidate and the parity seat is scored. A
faction-restricted run alternates the cross sides and tunes the faction's
seat.

**Control.** Each (scenario, seed, length) has one control game with no
overrides, cached and shared by all candidates.

**Measures:**

- **Margin:** the tuned seat's final unit count (0 if dead) minus the mean of
  its opponents' counts (0 each if dead).
- **Delta:** tuned margin − control margin. Above +10 is *better*, below −10
  is *worse*, otherwise *same*.
- **Decided win:** the game ended decided, with the seat alive and all others
  dead.
- **Fitness:** mean delta + 20 × (tuned decided-win rate − control rate),
  minus 0.3 × any increase in resource waste when waste scoring is on.

**Phases**, on **disjoint consecutive seed blocks** starting from a first
seed:

1. **Screening:** random candidates on short games.
2. **Confirmation:** the best few on fresh seeds and longer games.
3. **Four-player check** on a 4-player map (ARM, CORE, ARM, CORE), scored
   against the mean of the other three. It is reported but does not gate.
4. **Evaluate mode:** fixed named candidates run only on seeds never used
   before. Use it to split a winning candidate into single knobs before
   adopting any; a search winner carries passenger knobs.

**Candidates.** Each knob is drawn from a seeded generator, uniform or
log-uniform. A candidate's id is a hash of its sorted knob values.

**Resume.** Each game is appended to a results log keyed by (scenario,
candidate, seed, arm, length). A configuration fingerprint (binary, data, map,
difficulty, start mode, first seed, faction) refuses mismatched resumes.

### 8.3 Statistics

- **Wilson 95% interval** on the fraction of *better* games, with z = 1.96.
  Ties count as failures: n is all games. With p̂ = better/n:
  - centre = (p̂ + z²/2n) / (1 + z²/n);
  - half-width = z·√((p̂(1−p̂) + z²/4n)/n) / (1 + z²/n);
  - the interval is clamped to [0, 1], and n = 0 gives [0, 1].
- **Sign test:** exact two-sided binomial at p = 0.5 over better versus worse,
  with ties excluded. With n = better + worse and k = min(better, worse), p =
  min(1, 2·Σᵢ₌₀ᵏ C(n,i)/2ⁿ).
- **Report:** per candidate, n, better/same/worse, mean delta, Wilson [lo, hi]
  to 2 places, sign p to 3 places, decided delta, fitness, and the mean delta
  per scenario. Candidates are ranked by fitness, then by the Wilson lower
  bound.
- **A second analyser** scores a pair as a win if only the tuned seat
  survives, a loss if it died and an opponent survived, and otherwise by army
  count difference beyond ±2. It uses win rate = (W + D/2)/n with a Wilson
  interval: better if the lower bound > 0.5, worse if the upper bound < 0.5.
  It adds a sign test over decisive pairs, a paired bootstrap (10000
  resamples) and a power estimate. Use either method, but say which.

### 8.4 Reported results

Self-play tuning (Great Divide, Standard, paired):

| Change | Better / same / worse | Mean delta | Sign p | Decided rate (tuned vs control) |
|---|---|---|---|---|
| ARM lab shares all 0 (seeds 101-116, 1800 s, 32 games) | 25 / 2 / 5 | +74.5 | < 0.001 | 0.50 vs 0.09 |
| ...all eight searched knobs | 29 / 0 / 3 | +71.0 | < 0.001 | |
| ...the other five knobs only | 10 / 10 / 12 | −11.7 | 0.83 | |
| CORE composition candidate (seeds 201+, 32 games) | 21 / 6 / 5 | +40.4 | 0.002 | 0.47 vs 0.22 |
| Maker budgeting experiment (seeds 701-706, 18 pairs) | 8 / 3 / 7 | −15.6 | 1.0 | 0.22 vs 0.33 |
| Engagement-estimate outpost gate (36 pairs) | 0 / 35 / 1 | | | |

In the four-player check, the ARM change was better in 6 of 8 and the CORE
candidate 5/1/2.

**Against the opponent** (the classic AI). These were A/B runs between
successive builds, 1800 s, 4 seeds per map per batch, on Great Divide and Dark
Side, with Coast To Coast for ferries. About 120 games are reported
explicitly as wins for this AI. The comparisons used time to close out the
game, typically 17000-30000 ticks (9-17 minutes), not win rate.

**Exceptions.** At least two reported games against the opponent were
timeouts, not wins: one Coast To Coast control game and one Coast To Coast
ferry-batch game. A pre-ferry baseline also had 3 timeouts in 8 games. No
Wilson or sign statistic was ever computed against the opponent. Many arms
were byte-identical games, because the opponent never triggered the feature.
So "beat the classic AI in every reported game" is not literally supported:
it won every *decided* game reported, and timed out in a few on water maps.

Features that were neutral against the opponent were kept on for human play,
because the opponent never raids, never exposes its commander and rarely uses
air:

- threat sectors;
- proportional defence;
- reserve;
- raid sync;
- air sync;
- commander hunt.

The features that measurably mattered were:

- the composition shares;
- the ferry urgency on water maps;
- turning off focus fire and queue depth 2, both of which were regressions.

---

## 9. Known gaps

The **Source** column says where each gap comes from:

- **code**: observed in the code.
- **docs**: stated in the project's docs.
- **both**: stated in the docs and confirmed in the code.

| Gap | Source |
|---|---|
| One army: a single wave, reserve, raid party and guard group. No platoons; one intruder and one outpost answered per pass. | both |
| Weak on island and water maps: ferry only in Assault, one unit per air trip, no escorts, no return trips, no reinforcement ferries; island builder ferry without landing or threat checks; a land-route test that is pessimistic on diagonal passes; timeouts on Coast To Coast. | both |
| AI state not saved; the AI re-plans from scratch on load. | both |
| Only one builder is planned per pass (1 s at Standard). | code |
| The recent-loss list holds only buildings, so a raid that kills only units leaves no facing signal. | code |
| Tower facing falls back to the map centre before any enemy is seen. | code |
| Placement and feature reads leak through fog (§1.4). | code |
| The land target search ignores weapon eligibility. | code |
| D-gun friendly-fire check ignores allies. Radar contacts may include allies. | code |
| Wave recall dissolves the wave rather than bringing it home as a group. | code |
| The tier-two reserve start time is latched once and never reset. | code |
| A hover plant flagged as having no site is never retried. | code |
| Several gates count only dry extractors (early shipyard, air plant, mid-list extractor), so all-wet maps rely on later rules. | code |
| Metal makers and most towers need a kbot lab, so a shipyard-only base never builds them. | code |
| Unrecognised factory types are treated as kbot labs. | code |
| Some per-target memories are never pruned (memory growth only). | code |
| The navy uses one shipyard as home, and water-body connectivity stands in for naval reachability. | code |
| Fighters have no chase leash. Home-defence bombing ignores AA. Torpedo planes ignore AA. | code |
| Scout planes avoid ground threat, not AA. A hurt scout with orders keeps going. | code |
| Role table hard-codes ARM/CORE ids; any other side resolves as ARM, and missing ids silently disable a role. | code |
| Difficulty changes amounts and cadence, not decision quality. A two-axis difficulty (ambition vs skill) is proposed, not built. | docs |
| Data-driven build orders were planned but never built; the build order is code. | docs |
| The architecture doc and the status page say an AI in a network game desyncs; the roadmap and the code say this is fixed with the fixed-depth buffer. The code wins. | code vs docs |
| The tuning doc says confirmation uses the best 4 on 16 seeds at 1800 s; the tool's defaults are 2, 12 and 900 s, so the runs used overrides. The CORE candidate is described as "not yet applied" in one place but is the shipped default. | code vs docs |
| Capture is never used. | code |
| The timed opening, focus fire, cohesion, maker budgeting, island army cap and naval-triggered attack all ship disabled. | code |

---

## Appendix A. Knob catalogue

Defaults are the Standard profile before faction defaults. These are the names
used in personality files and arena overrides. Types: `bool`, `int`, or
`number` (decimal). Knobs marked "(not exposed to personality files)" are
fixed in the profile.


#### Cadence

| Knob | Type | Default | Meaning |
|---|---|---|---|
| `cadence.plan_ticks` | int | 30 | Ticks between build-planning passes (one builder is given a job per pass). |
| `cadence.tactics_ticks` | int | 15 | Ticks between army, air and transport passes. |
| `cadence.scout_ticks` | int | 60 | Ticks between scout passes. |
| `cadence.threat_grid_ticks` | int | 30 | Ticks between influence-grid rebuilds (not exposed to personality files). |

#### Cheats

| Knob | Type | Default | Meaning |
|---|---|---|---|
| `cheat.full_vision` | bool | false | Ignore fog of war entirely. |
| `cheat.income_factor` | number | 1 | Multiplier on this player's metal and energy income, applied by the simulation. |

#### Opening

| Knob | Type | Default | Meaning |
|---|---|---|---|
| `open.mex` | int | 3 | Extractors wanted before anything but energy-stall relief. |
| `open.solar` | int | 4 | Solar collectors wanted in the opening; also the "opening energy" mark. |
| `open.timed` | bool | false | Enable the timed opening posture (deadline biases). |
| `open.factory_by_s` | int | 60 | Timed posture: first lab wanted from this time. |
| `open.constructor_by_s` | int | 150 | Timed posture: at least one constructor once this time passes. |
| `open.expand_by_s` | int | 213 | Timed posture: expansion extractors wanted from this time. |
| `open.expand_sites` | int | 10 | Timed posture: outlying economy sites to reach. |
| `open.first_wave_size` | int | 7 | Timed posture: size of the first wave. |
| `open.first_wave_by_s` | int | 450 | Timed posture: earliest time of the first wave. |

#### Economy

| Knob | Type | Default | Meaning |
|---|---|---|---|
| `eco.solar` | int | 10 | Solar collector target. |
| `eco.mex` | int | 8 | Extractor target used by the metal-short and mid-list rules. |
| `eco.solar_on_demand` | bool | true | Skip the solar target while energy is plentiful. |
| `eco.energy_rows` | bool | true | Lay solars in rows behind the base. |
| `eco.makers` | int | 2 | Metal-maker ceiling. |
| `eco.makers_starved` | int | 6 | Raised maker ceiling after a long metal-short/energy-rich spell. |
| `eco.makers_starved_passes` | int | 120 | Consecutive planning passes of that spell before the raise. |
| `eco.makers_surplus_max` | int | 0 | Extra makers allowed on a large energy surplus (0 = off). |
| `eco.maker_off_below_pct` | int | 25 | Switch makers off below this energy storage percent. |
| `eco.maker_on_above_pct` | int | 70 | Switch makers on above this energy storage percent. |
| `eco.maker_budget` | bool | false | Experimental per-maker energy budgeting. |
| `eco.metal_storage` | int | 2 | Metal storage target. |
| `eco.energy_storage` | int | 2 | Energy storage target. |
| `eco.tidal` | int | 6 | Tidal generator target on naval maps. |
| `eco.fusion` | int | 1 | Fusion target (tier two). |
| `eco.uw_fusion` | int | 1 | Underwater fusion target. |
| `eco.save_s` | int | 60 | Default save-up window for an unaffordable item. |
| `eco.save_tech_s` | int | 240 | Save-up window for tier-two and other big items. |
| `eco.upgrade_mex` | bool | true | Replace finished extractors with the tier-two extractor once patches run out. |
| `eco.upgrade_min_metal_frac` | number | 0.5 | Stock fraction of the upgrade's cost required before tearing an extractor down. |
| `eco.upgrade_timeout_s` | int | 180 | Abandon an upgrade after this long. |
| `eco.capacity_spend` | bool | true | Add production capacity when income outruns spending. |
| `eco.capacity_ratio` | number | 1.25 | Income must exceed demand by this factor. |
| `eco.capacity_s` | int | 20 | ...continuously for this long. |
| `eco.capacity_constructors` | int | 2 | Extra constructors while capacity is short. |
| `eco.capacity_factories` | int | 1 | Extra factories while capacity is short. |
| `eco.surplus_expand` | bool | true | Spend a full metal store on more factories and bigger fleets. |
| `eco.surplus_factory_step` | int | 20 | Metal income per extra surplus factory. |
| `eco.surplus_factory_cap` | int | 4 | Maximum surplus factories per kind. |
| `eco.surplus_fleet_mult` | int | 3 | Fleet target multiplier with a full store. |
| `eco.surplus_bomber_mult` | int | 3 | Bomber target multiplier with a full store. |
| `eco.extra_labs` | int | 1 | Extra level-one labs with a full store. |
| `eco.t2_reserve` | bool | true | Hold non-builder production while a tier-two economy item is owed. |
| `eco.t2_reserve_min_army` | int | 6 | ...only while the army is at least this big. |
| `eco.t2_reserve_lab_after_s` | int | 0 | ...and also for the advanced lab after this time (0 = never). |
| `eco.t2_reserve_max_s` | int | 480 | ...for at most this long per game. |

#### Build

| Knob | Type | Default | Meaning |
|---|---|---|---|
| `build.constructors` | int | 1 | Base level-one constructor target. |
| `build.expansion_constructors` | int | 2 | Extra constructors for free deposits. |
| `build.deposits_per_constructor` | int | 4 | Free deposits per extra constructor. |
| `build.adv_constructors` | int | 1 | Advanced constructor target. |
| `build.air_constructors` | int | 1 | Air constructor target. |
| `build.vehicle_plants` | int | 1 | Vehicle plant target. |
| `build.vehicle_plant_first` | bool | true | On land maps build the vehicle plant before the air plant. |
| `build.air_plants` | int | 1 | Air plant target. |
| `build.adv_air_plants` | int | 1 | Advanced air plant target. |
| `build.repair_pads` | int | 1 | Air repair pad target. |
| `build.radar` | int | 1 | Radar target. |
| `build.adv_radar` | int | 1 | Advanced radar target. |
| `build.adv_labs` | int | 1 | Advanced lab target. |
| `build.hover_plants` | int | 0 | Hovercraft plant target (0 = off). |
| `build.queue_depth` | int | 1 | Top a factory up only when its queue holds fewer entries than this. |
| `build.drop_stalled_head` | bool | true | Drop a queue head that cannot progress. |
| `build.stall_s` | int | 60 | ...after this long. |
| `build.failed_site_s` | int | 120 | Remember a site whose order was dropped for this long. |
| `build.claim_radius` | number | 96 | Two builders may not claim extractor sites closer than this. |
| `build.near_mex_radius` | number | 1200 | First extractor search radius, around the builder. |
| `build.site_radius` | number | 512 | First radius of the generic site search. |
| `build.site_fallback_radius` | number | 2048 | Widened radius when nothing is found. |
| `build.expand_radius` | number | 4096 | Expansion extractor search radius from the base. |
| `build.commander_mex_radius` | number | 1600 | Commander's extractor search radius. |
| `build.commander_leash` | number | 1200 | Commander's leash from base while a constructor exists. |
| `build.commander_near_sites` | bool | true | Apply that leash. |
| `build.commander_assist_radius` | number | 800 | Commander helps finish frames within this radius. |
| `build.mex_avoid_guns_radius` | number | 450 | Refuse extractor sites with a known armed ground enemy this close. |
| `build.contested_radius` | number | 450 | Refuse other sites with an enemy or a recent loss this close. |
| `build.expand_our_side` | bool | true | Expand only to deposits nearer our base than theirs. |
| `build.expand_middle_if_boxed` | bool | true | ...unless that rule leaves nothing. |
| `build.expand_explored_only` | bool | false | Expand only to explored deposits (off: every spot is public). |
| `build.harass_radius` | number | 400 | Radius for factory-harassment and "under guns" tests. |
| `build.notice_harassment` | bool | true | Detect frames being killed on a factory pad. |
| `build.enemy_gun_range_from_weapon` | bool | true | Use an enemy tower's real reach for the "under guns" test. |
| `build.enemy_gun_margin` | number | 32 | Margin added to that reach. |
| `build.site_guard_size` | int | 0 | Units sent to guard a far build site (0 = off). |
| `build.site_guard_min_distance` | number | 900 | Only for sites at least this far from base. |
| `build.site_guard_threat` | number | 0 | Influence-grid threat needed near the site (0 = always). |
| `build.site_guard_threat_radius` | number | 600 | Radius of that threat sum. |
| `build.site_guard_timeout_s` | int | 180 | Release a guard after this long. |
| `build.mission_isolated_sea_air` | int | 0 | 1: skip land factories on an isolated start; 2: skip the lab too (0 = off). |
| `build.isolated_army_cap` | int | 0 | Cap the land army on island maps (0 = off). |
| `build.isolated_cap_water_frac` | number | 0.8 | ...on maps at least this wet. |

#### Repair

| Knob | Type | Default | Meaning |
|---|---|---|---|
| `build.repair` | bool | true | Repair damaged defences and factories. |
| `build.repair_below_pct` | int | 90 | Repair below this health percent. |
| `build.repairers` | int | 2 | Repairers per structure. |
| `build.repair_radius` | number | 800 | Search radius from the builder. |
| `build.repair_under_fire` | bool | false | Allow repairs under enemy guns. |
| `build.repair_commander` | bool | true | Send builders to repair the commander. |
| `build.repair_commander_below_pct` | int | 90 | ...below this health percent. |
| `build.commander_repairers` | int | 2 | ...this many. |
| `build.repair_commander_radius` | number | 1200 | ...from within this radius. |
| `build.mend_units` | bool | true | A builder heals damaged mobile units at base. |
| `build.mend_below_pct` | int | 50 | Mend units below this health percent. |
| `build.mend_radius` | number | 700 | ...within this radius of base. |
| `build.commander_mends` | bool | false | The commander may mend too. |

#### Reclaim

| Knob | Type | Default | Meaning |
|---|---|---|---|
| `reclaim.field_escort` | int | 4 | Wave size needed before field reclaim. |
| `reclaim.field_batch` | int | 6 | Wrecks queued per field-reclaim trip. |
| `reclaim.field_radius` | number | 700 | Wreck search radius round the wave. |
| `reclaim.own_reclaimers` | int | 1 | Builders clearing our own wrecks at once. |
| `reclaim.own_delay_s` | int | 10 | Wait after the loss. |
| `reclaim.own_batch` | int | 4 | Wrecks per trip. |
| `reclaim.own_only_when_short` | bool | false | Only while metal is short. |

#### Safety

| Knob | Type | Default | Meaning |
|---|---|---|---|
| `safety.builders` | bool | true | Pull exposed builders back. |
| `safety.margin` | number | 150 | Added to a mobile enemy's reach. |
| `safety.cover_radius` | number | 500 | Own units within this radius count as cover. |
| `safety.cover_ratio` | number | 1 | Cover metal needed per threat metal. |
| `safety.finish_above_pct` | int | 70 | A builder stays if its frame is this complete. |
| `safety.shelter_s` | int | 10 | Do not re-task a retreated builder for this long. |
| `safety.resume` | bool | true | Queue the interrupted job behind the retreat. |
| `safety.contact_memory_ticks` | int | 150 | Ignore contacts not seen for this many ticks. |

#### Defence

| Knob | Type | Default | Meaning |
|---|---|---|---|
| `def.towers` | int | 2 | Light laser towers at base. |
| `def.heavy_towers` | int | 2 | Heavy towers (tier two). |
| `def.base_aa_towers` | int | 1 | Anti-air towers with no air seen. |
| `def.reactive_aa_towers` | int | 3 | Anti-air towers once air is seen. |
| `def.mobile_aa` | int | 2 | Mobile anti-air units. |
| `def.aa_escorts` | int | 1 | Mobile AA guarding the wave. |
| `def.distance` | number | 160 | Default tower post distance. |
| `def.radar_distance` | number | 320 | Radar post distance. |
| `def.spread` | bool | true | Use the spaced, scored tower siting. |
| `def.perimeter_first` | bool | true | Push the first towers out to the base's edge. |
| `def.perimeter_count` | int | 3 | ...for this many towers. |
| `def.perimeter_margin` | number | 160 | ...this far past the outermost building. |
| `def.face_losses` | bool | true | Face towers where buildings were recently lost. |
| `def.face_threat_sector` | bool | true | Face towers at the strongest threat sector. |
| `def.payback_s` | int | 90 | A tower may cost this many seconds of income (0 = always). |
| `def.payback_per_mex_s` | int | 45 | ...plus this many per extractor it covers. |
| `def.outpost_towers` | int | 3 | Base number of outpost towers. |
| `def.outpost_income_step` | int | 8 | One more outpost tower per this much metal income (0 = fixed). |
| `def.outpost_max` | int | 8 | Outpost tower cap. |
| `def.outpost_min_mex` | int | 2 | Uncovered extractors needed to justify an outpost tower. |
| `def.outpost_towers_to_constructors` | bool | true | Commander leaves outpost towers to constructors. |
| `def.raid_memory_s` | int | 300 | Remember raided extractor sites this long. |
| `def.rebuild` | bool | true | Rebuild lost towers in place. |
| `def.rebuild_delay_s` | int | 20 | ...after this wait. |
| `def.rebuild_max` | int | 3 | ...at most this many times per site. |
| `def.rebuild_memory_s` | int | 1200 | ...within this memory. |

#### Fortify

| Knob | Type | Default | Meaning |
|---|---|---|---|
| `fort.every_tower` | bool | false | Wall every light tower. |
| `fort.at_tier_two` | bool | true | ...once an advanced lab stands. |
| `fort.where_attacked` | bool | true | Wall towers on the side they are repeatedly hit from. |
| `fort.rebuilt` | bool | true | Wall rebuilt towers. |
| `fort.aa_twice_lost` | bool | true | Add an AA tower behind a site lost twice. |
| `fort.aa_tower` | bool | true | Add an AA tower behind walled towers. |
| `fort.wrap` | bool | true | Hug the tower with walls (otherwise a line in front). |
| `fort.wrap_gap_tiles` | int | 0 | Gap between tower and wall ring. |
| `fort.wrap_walls` | int | 5 | Walls per side in wrap mode. |
| `fort.walls_per_tower` | int | 5 | Walls per tower when walling every tower. |
| `fort.reactive_walls` | int | 3 | Walls per side in line mode. |
| `fort.line_distance` | number | 220 | Line distance in front of the tower. |
| `fort.aa_back` | number | 96 | AA tower distance behind. |
| `fort.aa_cover_radius` | number | 200 | An AA tower this close already covers it. |
| `fort.attacker_radius` | number | 500 | Attackers within this radius of a hit tower are blamed. |
| `fort.repeat_attacks` | int | 2 | Attack episodes from one direction before walling. |
| `fort.attack_gap_s` | int | 20 | Hits closer than this are one episode. |
| `fort.attack_memory_s` | int | 600 | Forget episodes after this. |
| `fort.extra_constructors` | int | 1 | Extra constructors while walls are owed. |

#### Mix

| Knob | Type | Default | Meaning |
|---|---|---|---|
| `mix.lab_raider` | int | 2 | Lab share: raider kbot. |
| `mix.lab_rocket` | int | 1 | Lab share: rocket kbot. |
| `mix.lab_artillery` | int | 0 | Lab share: artillery kbot. |
| `mix.vp_light_tank` | int | 1 | Vehicle plant share: light tank. |
| `mix.vp_missile` | int | 0 | Vehicle plant share: missile truck. |
| `mix.vp_medium_tank` | int | 0 | Vehicle plant share: medium tank. |
| `mix.counter` | bool | true | Lean the shares against the observed enemy. |
| `mix.counter_bonus` | int | 2 | Share points added by the static/mobile rule. |
| `mix.counter_trigger` | number | 0.3 | Fraction of the enemy army that triggers a lean. |
| `mix.memory` | bool | true | Keep a decaying role histogram of the enemy. |
| `mix.memory_halflife_s` | int | 30 | Halve the histogram this often. |
| `mix.commit_s` | int | 60 | Recompute the lean at most this often. |
| `mix.lean` | int | 2 | Share points a triggered role adds. |

#### Tech

| Knob | Type | Default | Meaning |
|---|---|---|---|
| `tech.enabled` | bool | true | Go to tier two when it pays. |
| `tech.min_income` | int | 10 | Metal income needed. |
| `tech.min_value_ratio` | number | 1.5 | Tier-two assault unit must be worth this much more per metal. |

#### Air

| Knob | Type | Default | Meaning |
|---|---|---|---|
| `air.fighters` | int | 2 | Fighter floor. |
| `air.fighters_max` | int | 8 | Fighter ceiling when answering enemy air. |
| `air.bombers` | int | 4 | Bomber target. |
| `air.gunships` | int | 3 | Gunship target. |
| `air.scout_planes` | int | 1 | Scout plane target. |
| `air.transports` | int | 1 | Air transport target (not exposed to personality files). |
| `air.patrol_fighters` | int | 2 | Fighters on standing patrol. |
| `air.patrol_bombers` | int | 2 | Bombers on standing patrol. |
| `air.patrol_margin` | number | 400 | Patrol arc distance past the base edge. |
| `air.leash` | number | 2400 | Idle aircraft further than this return. |
| `air.worth_it_defences` | int | 4 | Enemy armed buildings that make air worth teching. |
| `air.sync` | bool | true | Hold bombers and gunships for the ground wave. |
| `air.sync_radius` | number | 600 | Release radius. |
| `air.bomb_home_radius` | number | 900 | Bomb intruders within this radius of base first. |
| `air.bomb_max_aa` | int | 2 | Skip targets under more AA than this. |
| `air.bomb_factory_weight` | number | 2 | Score multiplier for factories. |
| `air.bomb_aa_penalty` | number | 1 | Score divisor per AA unit. |
| `air.bomb_distance_scale` | number | 2400 | Distance scale in the score. |
| `air.bomb_army_reach` | number | 700 | Targets this close to the wave are discounted. |
| `air.bomb_army_discount` | number | 0.25 | ...by this factor. |
| `air.bomb_cluster_radius` | number | 300 | Cluster radius for mobile targets. |
| `air.bomb_cluster_min` | int | 3 | Minimum cluster size. |
| `air.gunship_pack` | int | 2 | Gunships needed before they fly. |
| `air.gunship_radius` | number | 1400 | Gunships support the wave within this radius. |
| `air.torpedo_planes` | int | 6 | Torpedo seaplane target. |

#### Scout

| Knob | Type | Default | Meaning |
|---|---|---|---|
| `scout.enabled` | int | 1 | 0 disables scouting (not exposed to personality files). |
| `scout.vehicles` | int | 1 | Scout vehicle target. |
| `scout.naval` | bool | true | Borrow a hull to scout when nothing enemy is known. |

#### Army

| Knob | Type | Default | Meaning |
|---|---|---|---|
| `army.waves` | bool | true | Attack in waves (otherwise the whole army). |
| `army.wave_size` | int | 8 | Army size that launches an attack. |
| `army.spent_size` | int | 3 | A wave smaller than this is spent. |
| `army.reinforce` | int | 3 | Join the wave in groups of at least this. |
| `army.patience_s` | int | 240 | Attack anyway after this long without attacking. |
| `army.odds` | bool | true | Also attack when our army outvalues theirs. |
| `army.odds_open` | number | 1.1 | Ratio that opens the odds gate. |
| `army.odds_close` | number | 0.7 | Ratio that closes it. |
| `army.reserve` | bool | true | Hold part of the wave back. |
| `army.commit_frac` | number | 0.7 | Fraction committed at formation. |
| `army.reserve_release_s` | int | 20 | Release the reserve after this much contact. |
| `army.attack_move` | bool | true | Waves use attack-move. |
| `army.cohesion` | number | 0 | Leaders wait for the pack beyond this (0 = off). |
| `army.meet_count` | int | 3 | Enemy units that divert the wave to fight them. |
| `army.meet_radius` | number | 900 | ...within this radius of the wave. |
| `army.target_base_radius` | number | 900 | Wave objectives are kept within this radius of the enemy base. |
| `army.threat_aversion` | number | 1 | Weight of enemy firepower against value in objective scoring. |
| `army.rally` | number | 220 | Rally point distance from base towards the enemy. |
| `army.engage` | number | 450 | Base acquisition radius. |
| `army.acquire_at_range` | bool | true | Acquire at own weapon reach when longer. |
| `army.kite` | bool | true | Back off from shorter-ranged mobile targets. |
| `army.kite_margin` | number | 40 | Margin in the kiting and acquisition rules. |
| `army.blocked_shots` | bool | true | Move when terrain blocks a direct shot. |
| `army.stalls` | bool | true | Reposition when a target takes no damage. |
| `army.stall_s` | int | 15 | No-damage time before repositioning. |
| `army.stall_sidestep` | number | 240 | Sidestep distance. |
| `army.stall_tries` | int | 2 | Repositions before giving up. |
| `army.stall_forget_s` | int | 60 | Give up on the target for this long. |
| `army.focus_fire` | bool | false | Wave focuses one target (off). |
| `army.focus_latch_s` | int | 5 | Focus target held this long. |
| `army.one_enemy` | bool | true | Fight one enemy player at a time. |
| `army.enemy_switch_margin` | number | 600 | Switch enemy only if another is this much nearer. |
| `army.raids` | bool | true | Send a raiding party while a wave is out. |
| `army.raid_size` | int | 3 | Raiding party size. |
| `army.raid_avoid_base` | number | 900 | Raids avoid this radius around the enemy base. |
| `army.raid_sync` | bool | true | Hold the raid until the wave engages. |
| `army.raid_sync_s` | int | 15 | ...for at most this long. |
| `army.raid_frames` | bool | true | Prefer fresh enemy expansion frames and their builders. |
| `army.raid_frame_window_s` | int | 30 | A frame is fresh this long after first sight. |
| `army.hunt_commander` | bool | true | Make the enemy commander the objective. |
| `army.hunt_exposed_only` | bool | true | ...only when it is exposed. |
| `army.hunt_min_distance` | number | 600 | Exposed: this far from its base. |
| `army.hunt_escort_radius` | number | 450 | Escort counted within this radius. |
| `army.hunt_max_escort` | int | 600 | Exposed: escort worth less than this. |
| `army.hunt_latch_s` | int | 10 | Exposure verdict held this long. |

#### Defence response

| Knob | Type | Default | Meaning |
|---|---|---|---|
| `army.defend_radius` | number | 900 | Enemies inside this radius of base are intruders. |
| `army.hold_outnumbered` | bool | true | Do not charge intruders that outnumber us. |
| `army.reserve_defends` | bool | true | Units at home answer intruders while the wave stays out. |
| `army.proportional` | bool | true | Send only enough to beat the intruders. |
| `army.respond_ratio` | number | 1.3 | Metal sent per intruder metal. |
| `army.respond_min` | int | 1 | Minimum responders. |
| `army.respond_max_share` | number | 1 | Maximum share of home units sent. |
| `army.recall_ratio` | number | 1.5 | Recall the wave when base threat exceeds wave value by this. |
| `army.sectors` | bool | true | Keep the 16-sector threat memory. |
| `army.sector_halflife_s` | int | 60 | Halve sector threat this often. |
| `army.radar_rings` | number | 2.5 | Radar warning ring, in multiples of the defend radius. |
| `army.radar_min_contacts` | int | 2 | Closing contacts that raise the warning. |
| `army.outpost_response` | bool | true | Answer raids on outlying buildings. |
| `army.outpost_raid_radius` | number | 350 | Raider search radius at the outpost. |
| `army.outpost_response_radius` | number | 1500 | Responders within this radius. |
| `army.outpost_strength` | number | 1.2 | Responder metal per raider metal. |
| `army.outpost_predictor` | bool | false | Use the engagement estimate instead (off). |
| `army.outpost_predictor_margin` | number | 0 | Estimate margin needed. |

#### Units

| Knob | Type | Default | Meaning |
|---|---|---|---|
| `army.retreat_damaged` | bool | true | Send damaged units home to be mended. |
| `army.retreat_raider_pct` | int | 30 | Raider retreat health percent. |
| `army.retreat_line_pct` | int | 45 | Other units' retreat health percent. |
| `army.rejoin_pct` | int | 60 | Rejoin above this health percent. |
| `army.mend_wait_s` | int | 45 | Wait at most this long for mending. |
| `army.mend_needs_mender` | bool | true | Only retreat if a builder exists. |
| `army.mend_max_walk` | number | 1800 | Only retreat from within this distance. |
| `army.mend_spot_radius` | number | 160 | Mend spots ring radius. |
| `army.mend_spot_tolerance` | number | 400 | Move only if further than this from the spot. |

#### Commander

| Knob | Type | Default | Meaning |
|---|---|---|---|
| `cmd.stands_ground` | bool | true | Commander fights small threats. |
| `cmd.fight_max_metal` | int | 600 | ...worth up to this; also its value as builder cover. |
| `cmd.retreat_pct` | int | 50 | ...while above this health percent. |
| `cmd.danger_radius` | number | 450 | Threat radius round the commander. |
| `cmd.guard_radius` | number | 1500 | Army units within this radius defend it. |
| `cmd.dgun` | bool | true | Use the commander's special weapon. |
| `cmd.dgun_by_value` | bool | true | Aim it at the most valuable target. |
| `cmd.keeps_frames` | bool | true | Do not abandon a frame it would lose. |
| `cmd.frame_absence_s` | int | 20 | Frame life below which it stays. |
| `cmd.frame_cover_radius` | number | 800 | Own cover counted within this radius. |
| `cmd.frame_handover_radius` | number | 800 | Hand the frame to a builder within this radius. |
| `cmd.shuns_ships` | bool | true | Flee rather than fight ships. |
| `cmd.answers_harassment` | bool | false | Commander attacks factory harassers when there is no army (off). |
| `cmd.defends_alone_max` | int | 1 | Commander alone fights up to this many intruders. |
| `cmd.own_reachability` | bool | true | Plan the commander's sites on its own movement class. |

#### Navy

| Knob | Type | Default | Meaning |
|---|---|---|---|
| `navy.fleet` | int | 6 | Warship target on water maps (half on mixed maps). |
| `navy.shipyards` | int | 2 | Shipyard target. |
| `navy.early_shipyard` | bool | true | Allow the shipyard early in the list. |
| `navy.ship_builders_plan_base` | bool | false | Construction ships use the base list (off). |
| `navy.yard_spacing` | number | 448 | Minimum spacing between our yards. |
| `navy.scout_ships` | int | 1 | Scout ship target. |
| `navy.sea_transports` | int | 1 | Sea transport target. |
| `navy.construction_ships` | int | 2 | Construction ship target. |
| `navy.subs` | int | 2 | Submarine target. |
| `navy.subs_after_destroyers` | int | 3 | Destroyers needed before submarines. |
| `navy.adv_yards` | int | 1 | Advanced shipyard target. |
| `navy.tech_income` | int | 15 | Income for the advanced shipyard. |
| `navy.cruisers` | int | 4 | Cruiser target. |
| `navy.battleships` | int | 2 | Battleship target. |
| `navy.aa_ships` | int | 2 | AA ship target. |
| `navy.seaplane_plants` | int | 1 | Seaplane plant target. |
| `navy.sonar` | int | 1 | Sonar target. |
| `navy.torpedo_launchers` | int | 1 | Torpedo launcher target. |
| `navy.sail_size` | int | 3 | Hulls gathered before the fleet sails. |
| `navy.attack_size` | int | 0 | Fleet size that also triggers the land attack (0 = off). |
| `navy.rally` | number | 320 | Fleet station distance from the yard. |
| `navy.stall_s` | int | 12 | Naval no-damage time before repositioning. |
| `navy.stand_off` | bool | true | Outranging hulls hold off. |

#### Ferry

| Knob | Type | Default | Meaning |
|---|---|---|---|
| `ferry.urgent` | bool | true | Prioritise the first transport when the enemy is across water. |
| `ferry.first_by_s` | int | 480 | After this time the transport outranks the scout too. |
| `ferry.min_party` | int | 6 | Army size before a sea ferry sails. |
| `ferry.want_from_map` | bool | true | Decide ferry need from the start positions and remember it. |
| `ferry.muster` | bool | true | Gather passengers on a shore point. |
| `ferry.landing_fan` | bool | true | Search landing points on a fan of bearings. |
| `ferry.landing_avoid_threat` | bool | true | Score landing points by threat. |
| `ferry.landing_threat_radius` | number | 300 | Threat radius for that score. |
| `ferry.landing_rings` | int | 24 | Landing search rings. |
| `ferry.refusal_cell` | int | 192 | Grid size for remembering failed landings. |
| `ferry.refusal_s` | int | 30 | Remember a failed landing this long. |
| `ferry.stall_s` | int | 20 | A loaded hull making no progress this long re-plans. |
| `ferry.pickup_timeout_s` | int | 120 | Abandon a pickup after this long (not exposed to personality files). |

#### Misc

| Knob | Type | Default | Meaning |
|---|---|---|---|
| `misc.idle` | bool | false | The seat does nothing (difficulty "idle"). |

---

## Appendix B. Role table (stock ARM / CORE ids)

The side name is compared ignoring case. CORE gets the CORE column, and any
other side gets the ARM column. Each id is checked against the loaded unit
definitions; an id that is missing leaves its role empty, and every rule that
names an empty role does nothing. The air repair pad role is also emptied
unless its definition is both an air base and a builder. The scout roles are
the scout plane and the scout vehicle. "Mobile anti-air" in §3.2 means the
anti-air kbot role. For a mod, fill this table with that mod's equivalents.

| Role | ARM | CORE |
|---|---|---|
| level-one metal extractor | ARMMEX | CORMEX |
| solar collector | ARMSOLAR | CORSOLAR |
| first kbot lab | ARMLAB | CORLAB |
| level-one constructor (kbot) | ARMCK | CORCK |
| raider kbot | ARMPW | CORAK |
| rocket kbot | ARMROCK | CORSTORM |
| light laser tower | ARMLLT | CORLLT |
| radar | ARMRAD | CORRAD |
| metal maker (land) | ARMMAKR | CORMAKR |
| air plant | ARMAP | CORAP |
| scout plane | ARMPEEP | CORFINK |
| air transport | ARMATLAS | CORVALK |
| fighter | ARMFIG | CORVENG |
| bomber | ARMTHUND | CORSHAD |
| advanced air plant | ARMAAP | CORAAP |
| gunship | ARMBRAWL | CORAPE |
| air repair pad | ARMASP | CORASP |
| air constructor | ARMCA | CORCA |
| vehicle plant | ARMVP | CORVP |
| scout vehicle | ARMFAV | CORFAV |
| light tank | ARMFLASH | CORGATOR |
| missile truck | ARMSAM | CORMIST |
| medium tank | ARMSTUMP | CORRAID |
| artillery kbot | ARMHAM | CORTHUD |
| wall (dragon's teeth) | ARMDRAG | CORDRAG |
| anti-air tower | ARMRL | CORRL |
| anti-air kbot | ARMJETH | CORCRASH |
| advanced kbot lab | ARMALAB | CORALAB |
| advanced constructor | ARMACK | CORACK |
| tier-two assault kbot | ARMZEUS | CORCAN |
| heavy laser tower | ARMHLT | CORHLT |
| heavy plasma tower | ARMGUARD | CORPUN |
| advanced radar | ARMARAD | CORARAD |
| tier-two extractor | ARMMOHO | CORMOHO |
| fusion plant | ARMFUS | CORFUS |
| shipyard | ARMSY | CORSY |
| construction ship | ARMCS | CORCS |
| scout ship | ARMPT | CORPT |
| destroyer | ARMROY | CORROY |
| sea transport | ARMTSHIP | CORTSHIP |
| submarine | ARMSUB | CORSUB |
| advanced shipyard | ARMASY | CORASY |
| cruiser | ARMCRUS | CORCRUS |
| battleship | ARMBATS | CORBATS |
| anti-air ship | ARMAAS | CORARCH |
| advanced construction sub | ARMACSUB | CORACSUB |
| seaplane plant | ARMPLAT | CORPLAT |
| seaplane fighter | ARMSFIG | CORSFIG |
| torpedo seaplane | ARMSEAP | CORSEAP |
| hover plant | ARMHP | CORHP |
| hover tank | ARMANAC | CORSNAP |
| hover anti-air | ARMAH | CORAH |
| tidal generator | ARMTIDE | CORTIDE |
| sonar | ARMSONAR | CORSONAR |
| torpedo launcher | ARMTL | CORTL |
| underwater extractor | ARMUWMEX | CORUWMEX |
| floating metal maker | ARMFMKR | CORFMKR |
| geothermal plant | ARMGEO | CORGEO |
| metal storage | ARMMSTOR | CORMSTOR |
| energy storage | ARMESTOR | CORESTOR |
| underwater metal storage | ARMUWMS | CORUWMS |
| underwater energy storage | ARMUWES | CORUWES |
| underwater fusion | ARMUWFUS | CORUWFUS |
