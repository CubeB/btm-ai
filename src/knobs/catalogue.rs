//! The knob catalogue of SPEC.md Appendix A, generated from the table and then
//! maintained by hand. One struct per name prefix; see [`define_knobs`].

use super::{Fixed, KnobInfo, KnobType, Value};

define_knobs! {
    /// Knobs named `cadence.*`.
    cadence: CadenceKnobs {
        /// Ticks between build-planning passes (one builder is given a job per pass).
        ///
        /// Default: `30`.
        plan_ticks: i32 = 30, true;
        /// Ticks between army, air and transport passes.
        ///
        /// Default: `15`.
        tactics_ticks: i32 = 15, true;
        /// Ticks between scout passes.
        ///
        /// Default: `60`.
        scout_ticks: i32 = 60, true;
        /// Ticks between influence-grid rebuilds (not exposed to personality files).
        ///
        /// Default: `30`.
        threat_grid_ticks: i32 = 30, false;
    }
    /// Knobs named `cheat.*`.
    cheat: CheatKnobs {
        /// Ignore fog of war entirely.
        ///
        /// Default: `false`.
        full_vision: bool = false, true;
        /// Multiplier on this player's metal and energy income, applied by the simulation.
        ///
        /// Default: `1`.
        income_factor: Fixed = Fixed::from_raw(10000), true;
    }
    /// Knobs named `open.*`.
    open: OpenKnobs {
        /// Extractors wanted before anything but energy-stall relief.
        ///
        /// Default: `3`.
        mex: i32 = 3, true;
        /// Solar collectors wanted in the opening; also the "opening energy" mark.
        ///
        /// Default: `4`.
        solar: i32 = 4, true;
        /// Enable the timed opening posture (deadline biases).
        ///
        /// Default: `false`.
        timed: bool = false, true;
        /// Timed posture: first lab wanted from this time.
        ///
        /// Default: `60`.
        factory_by_s: i32 = 60, true;
        /// Timed posture: at least one constructor once this time passes.
        ///
        /// Default: `150`.
        constructor_by_s: i32 = 150, true;
        /// Timed posture: expansion extractors wanted from this time.
        ///
        /// Default: `213`.
        expand_by_s: i32 = 213, true;
        /// Timed posture: outlying economy sites to reach.
        ///
        /// Default: `10`.
        expand_sites: i32 = 10, true;
        /// Timed posture: size of the first wave.
        ///
        /// Default: `7`.
        first_wave_size: i32 = 7, true;
        /// Timed posture: earliest time of the first wave.
        ///
        /// Default: `450`.
        first_wave_by_s: i32 = 450, true;
    }
    /// Knobs named `eco.*`.
    eco: EcoKnobs {
        /// Solar collector target.
        ///
        /// Default: `10`.
        solar: i32 = 10, true;
        /// Extractor target used by the metal-short and mid-list rules.
        ///
        /// Default: `8`.
        mex: i32 = 8, true;
        /// Skip the solar target while energy is plentiful.
        ///
        /// Default: `true`.
        solar_on_demand: bool = true, true;
        /// Lay solars in rows behind the base.
        ///
        /// Default: `true`.
        energy_rows: bool = true, true;
        /// Metal-maker ceiling.
        ///
        /// Default: `2`.
        makers: i32 = 2, true;
        /// Raised maker ceiling after a long metal-short/energy-rich spell.
        ///
        /// Default: `6`.
        makers_starved: i32 = 6, true;
        /// Consecutive planning passes of that spell before the raise.
        ///
        /// Default: `120`.
        makers_starved_passes: i32 = 120, true;
        /// Extra makers allowed on a large energy surplus (0 = off).
        ///
        /// Default: `0`.
        makers_surplus_max: i32 = 0, true;
        /// Switch makers off below this energy storage percent.
        ///
        /// Default: `25`.
        maker_off_below_pct: i32 = 25, true;
        /// Switch makers on above this energy storage percent.
        ///
        /// Default: `70`.
        maker_on_above_pct: i32 = 70, true;
        /// Experimental per-maker energy budgeting.
        ///
        /// Default: `false`.
        maker_budget: bool = false, true;
        /// Metal storage target.
        ///
        /// Default: `2`.
        metal_storage: i32 = 2, true;
        /// Energy storage target.
        ///
        /// Default: `2`.
        energy_storage: i32 = 2, true;
        /// Tidal generator target on naval maps.
        ///
        /// Default: `6`.
        tidal: i32 = 6, true;
        /// Fusion target (tier two).
        ///
        /// Default: `1`.
        fusion: i32 = 1, true;
        /// Underwater fusion target.
        ///
        /// Default: `1`.
        uw_fusion: i32 = 1, true;
        /// Default save-up window for an unaffordable item.
        ///
        /// Default: `60`.
        save_s: i32 = 60, true;
        /// Save-up window for tier-two and other big items.
        ///
        /// Default: `240`.
        save_tech_s: i32 = 240, true;
        /// Replace finished extractors with the tier-two extractor once patches run out.
        ///
        /// Default: `true`.
        upgrade_mex: bool = true, true;
        /// Stock fraction of the upgrade's cost required before tearing an extractor down.
        ///
        /// Default: `0.5`.
        upgrade_min_metal_frac: Fixed = Fixed::from_raw(5000), true;
        /// Abandon an upgrade after this long.
        ///
        /// Default: `180`.
        upgrade_timeout_s: i32 = 180, true;
        /// Add production capacity when income outruns spending.
        ///
        /// Default: `true`.
        capacity_spend: bool = true, true;
        /// Income must exceed demand by this factor.
        ///
        /// Default: `1.25`.
        capacity_ratio: Fixed = Fixed::from_raw(12500), true;
        /// ...continuously for this long.
        ///
        /// Default: `20`.
        capacity_s: i32 = 20, true;
        /// Extra constructors while capacity is short.
        ///
        /// Default: `2`.
        capacity_constructors: i32 = 2, true;
        /// Extra factories while capacity is short.
        ///
        /// Default: `1`.
        capacity_factories: i32 = 1, true;
        /// Spend a full metal store on more factories and bigger fleets.
        ///
        /// Default: `true`.
        surplus_expand: bool = true, true;
        /// Metal income per extra surplus factory.
        ///
        /// Default: `20`.
        surplus_factory_step: i32 = 20, true;
        /// Maximum surplus factories per kind.
        ///
        /// Default: `4`.
        surplus_factory_cap: i32 = 4, true;
        /// Fleet target multiplier with a full store.
        ///
        /// Default: `3`.
        surplus_fleet_mult: i32 = 3, true;
        /// Bomber target multiplier with a full store.
        ///
        /// Default: `3`.
        surplus_bomber_mult: i32 = 3, true;
        /// Extra level-one labs with a full store.
        ///
        /// Default: `1`.
        extra_labs: i32 = 1, true;
        /// Hold non-builder production while a tier-two economy item is owed.
        ///
        /// Default: `true`.
        t2_reserve: bool = true, true;
        /// ...only while the army is at least this big.
        ///
        /// Default: `6`.
        t2_reserve_min_army: i32 = 6, true;
        /// ...and also for the advanced lab after this time (0 = never).
        ///
        /// Default: `0`.
        t2_reserve_lab_after_s: i32 = 0, true;
        /// ...for at most this long per game.
        ///
        /// Default: `480`.
        t2_reserve_max_s: i32 = 480, true;
    }
    /// Knobs named `build.*`.
    build: BuildKnobs {
        /// Base level-one constructor target.
        ///
        /// Default: `1`.
        constructors: i32 = 1, true;
        /// Extra constructors for free deposits.
        ///
        /// Default: `2`.
        expansion_constructors: i32 = 2, true;
        /// Free deposits per extra constructor.
        ///
        /// Default: `4`.
        deposits_per_constructor: i32 = 4, true;
        /// Advanced constructor target.
        ///
        /// Default: `1`.
        adv_constructors: i32 = 1, true;
        /// Air constructor target.
        ///
        /// Default: `1`.
        air_constructors: i32 = 1, true;
        /// Vehicle plant target.
        ///
        /// Default: `1`.
        vehicle_plants: i32 = 1, true;
        /// On land maps build the vehicle plant before the air plant.
        ///
        /// Default: `true`.
        vehicle_plant_first: bool = true, true;
        /// Air plant target.
        ///
        /// Default: `1`.
        air_plants: i32 = 1, true;
        /// Advanced air plant target.
        ///
        /// Default: `1`.
        adv_air_plants: i32 = 1, true;
        /// Air repair pad target.
        ///
        /// Default: `1`.
        repair_pads: i32 = 1, true;
        /// Radar target.
        ///
        /// Default: `1`.
        radar: i32 = 1, true;
        /// Advanced radar target.
        ///
        /// Default: `1`.
        adv_radar: i32 = 1, true;
        /// Advanced lab target.
        ///
        /// Default: `1`.
        adv_labs: i32 = 1, true;
        /// Hovercraft plant target (0 = off).
        ///
        /// Default: `0`.
        hover_plants: i32 = 0, true;
        /// Top a factory up only when its queue holds fewer entries than this.
        ///
        /// Default: `1`.
        queue_depth: i32 = 1, true;
        /// Drop a queue head that cannot progress.
        ///
        /// Default: `true`.
        drop_stalled_head: bool = true, true;
        /// ...after this long.
        ///
        /// Default: `60`.
        stall_s: i32 = 60, true;
        /// Remember a site whose order was dropped for this long.
        ///
        /// Default: `120`.
        failed_site_s: i32 = 120, true;
        /// Two builders may not claim extractor sites closer than this.
        ///
        /// Default: `96`.
        claim_radius: Fixed = Fixed::from_raw(960000), true;
        /// First extractor search radius, around the builder.
        ///
        /// Default: `1200`.
        near_mex_radius: Fixed = Fixed::from_raw(12000000), true;
        /// First radius of the generic site search.
        ///
        /// Default: `512`.
        site_radius: Fixed = Fixed::from_raw(5120000), true;
        /// Widened radius when nothing is found.
        ///
        /// Default: `2048`.
        site_fallback_radius: Fixed = Fixed::from_raw(20480000), true;
        /// Expansion extractor search radius from the base.
        ///
        /// Default: `4096`.
        expand_radius: Fixed = Fixed::from_raw(40960000), true;
        /// Commander's extractor search radius.
        ///
        /// Default: `1600`.
        commander_mex_radius: Fixed = Fixed::from_raw(16000000), true;
        /// Commander's leash from base while a constructor exists.
        ///
        /// Default: `1200`.
        commander_leash: Fixed = Fixed::from_raw(12000000), true;
        /// Apply that leash.
        ///
        /// Default: `true`.
        commander_near_sites: bool = true, true;
        /// Commander helps finish frames within this radius.
        ///
        /// Default: `800`.
        commander_assist_radius: Fixed = Fixed::from_raw(8000000), true;
        /// Refuse extractor sites with a known armed ground enemy this close.
        ///
        /// Default: `450`.
        mex_avoid_guns_radius: Fixed = Fixed::from_raw(4500000), true;
        /// Refuse other sites with an enemy or a recent loss this close.
        ///
        /// Default: `450`.
        contested_radius: Fixed = Fixed::from_raw(4500000), true;
        /// Expand only to deposits nearer our base than theirs.
        ///
        /// Default: `true`.
        expand_our_side: bool = true, true;
        /// ...unless that rule leaves nothing.
        ///
        /// Default: `true`.
        expand_middle_if_boxed: bool = true, true;
        /// Expand only to explored deposits (off: every spot is public).
        ///
        /// Default: `false`.
        expand_explored_only: bool = false, true;
        /// Radius for factory-harassment and "under guns" tests.
        ///
        /// Default: `400`.
        harass_radius: Fixed = Fixed::from_raw(4000000), true;
        /// Detect frames being killed on a factory pad.
        ///
        /// Default: `true`.
        notice_harassment: bool = true, true;
        /// Use an enemy tower's real reach for the "under guns" test.
        ///
        /// Default: `true`.
        enemy_gun_range_from_weapon: bool = true, true;
        /// Margin added to that reach.
        ///
        /// Default: `32`.
        enemy_gun_margin: Fixed = Fixed::from_raw(320000), true;
        /// Units sent to guard a far build site (0 = off).
        ///
        /// Default: `0`.
        site_guard_size: i32 = 0, true;
        /// Only for sites at least this far from base.
        ///
        /// Default: `900`.
        site_guard_min_distance: Fixed = Fixed::from_raw(9000000), true;
        /// Influence-grid threat needed near the site (0 = always).
        ///
        /// Default: `0`.
        site_guard_threat: Fixed = Fixed::from_raw(0), true;
        /// Radius of that threat sum.
        ///
        /// Default: `600`.
        site_guard_threat_radius: Fixed = Fixed::from_raw(6000000), true;
        /// Release a guard after this long.
        ///
        /// Default: `180`.
        site_guard_timeout_s: i32 = 180, true;
        /// 1: skip land factories on an isolated start; 2: skip the lab too (0 = off).
        ///
        /// Default: `0`.
        mission_isolated_sea_air: i32 = 0, true;
        /// Cap the land army on island maps (0 = off).
        ///
        /// Default: `0`.
        isolated_army_cap: i32 = 0, true;
        /// ...on maps at least this wet.
        ///
        /// Default: `0.8`.
        isolated_cap_water_frac: Fixed = Fixed::from_raw(8000), true;
        /// Repair damaged defences and factories.
        ///
        /// Default: `true`.
        repair: bool = true, true;
        /// Repair below this health percent.
        ///
        /// Default: `90`.
        repair_below_pct: i32 = 90, true;
        /// Repairers per structure.
        ///
        /// Default: `2`.
        repairers: i32 = 2, true;
        /// Search radius from the builder.
        ///
        /// Default: `800`.
        repair_radius: Fixed = Fixed::from_raw(8000000), true;
        /// Allow repairs under enemy guns.
        ///
        /// Default: `false`.
        repair_under_fire: bool = false, true;
        /// Send builders to repair the commander.
        ///
        /// Default: `true`.
        repair_commander: bool = true, true;
        /// ...below this health percent.
        ///
        /// Default: `90`.
        repair_commander_below_pct: i32 = 90, true;
        /// ...this many.
        ///
        /// Default: `2`.
        commander_repairers: i32 = 2, true;
        /// ...from within this radius.
        ///
        /// Default: `1200`.
        repair_commander_radius: Fixed = Fixed::from_raw(12000000), true;
        /// A builder heals damaged mobile units at base.
        ///
        /// Default: `true`.
        mend_units: bool = true, true;
        /// Mend units below this health percent.
        ///
        /// Default: `50`.
        mend_below_pct: i32 = 50, true;
        /// ...within this radius of base.
        ///
        /// Default: `700`.
        mend_radius: Fixed = Fixed::from_raw(7000000), true;
        /// The commander may mend too.
        ///
        /// Default: `false`.
        commander_mends: bool = false, true;
    }
    /// Knobs named `reclaim.*`.
    reclaim: ReclaimKnobs {
        /// Wave size needed before field reclaim.
        ///
        /// Default: `4`.
        field_escort: i32 = 4, true;
        /// Wrecks queued per field-reclaim trip.
        ///
        /// Default: `6`.
        field_batch: i32 = 6, true;
        /// Wreck search radius round the wave.
        ///
        /// Default: `700`.
        field_radius: Fixed = Fixed::from_raw(7000000), true;
        /// Builders clearing our own wrecks at once.
        ///
        /// Default: `1`.
        own_reclaimers: i32 = 1, true;
        /// Wait after the loss.
        ///
        /// Default: `10`.
        own_delay_s: i32 = 10, true;
        /// Wrecks per trip.
        ///
        /// Default: `4`.
        own_batch: i32 = 4, true;
        /// Only while metal is short.
        ///
        /// Default: `false`.
        own_only_when_short: bool = false, true;
    }
    /// Knobs named `safety.*`.
    safety: SafetyKnobs {
        /// Pull exposed builders back.
        ///
        /// Default: `true`.
        builders: bool = true, true;
        /// Added to a mobile enemy's reach.
        ///
        /// Default: `150`.
        margin: Fixed = Fixed::from_raw(1500000), true;
        /// Own units within this radius count as cover.
        ///
        /// Default: `500`.
        cover_radius: Fixed = Fixed::from_raw(5000000), true;
        /// Cover metal needed per threat metal.
        ///
        /// Default: `1`.
        cover_ratio: Fixed = Fixed::from_raw(10000), true;
        /// A builder stays if its frame is this complete.
        ///
        /// Default: `70`.
        finish_above_pct: i32 = 70, true;
        /// Do not re-task a retreated builder for this long.
        ///
        /// Default: `10`.
        shelter_s: i32 = 10, true;
        /// Queue the interrupted job behind the retreat.
        ///
        /// Default: `true`.
        resume: bool = true, true;
        /// Ignore contacts not seen for this many ticks.
        ///
        /// Default: `150`.
        contact_memory_ticks: i32 = 150, true;
    }
    /// Knobs named `def.*`.
    def: DefKnobs {
        /// Light laser towers at base.
        ///
        /// Default: `2`.
        towers: i32 = 2, true;
        /// Heavy towers (tier two).
        ///
        /// Default: `2`.
        heavy_towers: i32 = 2, true;
        /// Anti-air towers with no air seen.
        ///
        /// Default: `1`.
        base_aa_towers: i32 = 1, true;
        /// Anti-air towers once air is seen.
        ///
        /// Default: `3`.
        reactive_aa_towers: i32 = 3, true;
        /// Mobile anti-air units.
        ///
        /// Default: `2`.
        mobile_aa: i32 = 2, true;
        /// Mobile AA guarding the wave.
        ///
        /// Default: `1`.
        aa_escorts: i32 = 1, true;
        /// Default tower post distance.
        ///
        /// Default: `160`.
        distance: Fixed = Fixed::from_raw(1600000), true;
        /// Radar post distance.
        ///
        /// Default: `320`.
        radar_distance: Fixed = Fixed::from_raw(3200000), true;
        /// Use the spaced, scored tower siting.
        ///
        /// Default: `true`.
        spread: bool = true, true;
        /// Push the first towers out to the base's edge.
        ///
        /// Default: `true`.
        perimeter_first: bool = true, true;
        /// ...for this many towers.
        ///
        /// Default: `3`.
        perimeter_count: i32 = 3, true;
        /// ...this far past the outermost building.
        ///
        /// Default: `160`.
        perimeter_margin: Fixed = Fixed::from_raw(1600000), true;
        /// Face towers where buildings were recently lost.
        ///
        /// Default: `true`.
        face_losses: bool = true, true;
        /// Face towers at the strongest threat sector.
        ///
        /// Default: `true`.
        face_threat_sector: bool = true, true;
        /// A tower may cost this many seconds of income (0 = always).
        ///
        /// Default: `90`.
        payback_s: i32 = 90, true;
        /// ...plus this many per extractor it covers.
        ///
        /// Default: `45`.
        payback_per_mex_s: i32 = 45, true;
        /// Base number of outpost towers.
        ///
        /// Default: `3`.
        outpost_towers: i32 = 3, true;
        /// One more outpost tower per this much metal income (0 = fixed).
        ///
        /// Default: `8`.
        outpost_income_step: i32 = 8, true;
        /// Outpost tower cap.
        ///
        /// Default: `8`.
        outpost_max: i32 = 8, true;
        /// Uncovered extractors needed to justify an outpost tower.
        ///
        /// Default: `2`.
        outpost_min_mex: i32 = 2, true;
        /// Commander leaves outpost towers to constructors.
        ///
        /// Default: `true`.
        outpost_towers_to_constructors: bool = true, true;
        /// Remember raided extractor sites this long.
        ///
        /// Default: `300`.
        raid_memory_s: i32 = 300, true;
        /// Rebuild lost towers in place.
        ///
        /// Default: `true`.
        rebuild: bool = true, true;
        /// ...after this wait.
        ///
        /// Default: `20`.
        rebuild_delay_s: i32 = 20, true;
        /// ...at most this many times per site.
        ///
        /// Default: `3`.
        rebuild_max: i32 = 3, true;
        /// ...within this memory.
        ///
        /// Default: `1200`.
        rebuild_memory_s: i32 = 1200, true;
    }
    /// Knobs named `fort.*`.
    fort: FortKnobs {
        /// Wall every light tower.
        ///
        /// Default: `false`.
        every_tower: bool = false, true;
        /// ...once an advanced lab stands.
        ///
        /// Default: `true`.
        at_tier_two: bool = true, true;
        /// Wall towers on the side they are repeatedly hit from.
        ///
        /// Default: `true`.
        where_attacked: bool = true, true;
        /// Wall rebuilt towers.
        ///
        /// Default: `true`.
        rebuilt: bool = true, true;
        /// Add an AA tower behind a site lost twice.
        ///
        /// Default: `true`.
        aa_twice_lost: bool = true, true;
        /// Add an AA tower behind walled towers.
        ///
        /// Default: `true`.
        aa_tower: bool = true, true;
        /// Hug the tower with walls (otherwise a line in front).
        ///
        /// Default: `true`.
        wrap: bool = true, true;
        /// Gap between tower and wall ring.
        ///
        /// Default: `0`.
        wrap_gap_tiles: i32 = 0, true;
        /// Walls per side in wrap mode.
        ///
        /// Default: `5`.
        wrap_walls: i32 = 5, true;
        /// Walls per tower when walling every tower.
        ///
        /// Default: `5`.
        walls_per_tower: i32 = 5, true;
        /// Walls per side in line mode.
        ///
        /// Default: `3`.
        reactive_walls: i32 = 3, true;
        /// Line distance in front of the tower.
        ///
        /// Default: `220`.
        line_distance: Fixed = Fixed::from_raw(2200000), true;
        /// AA tower distance behind.
        ///
        /// Default: `96`.
        aa_back: Fixed = Fixed::from_raw(960000), true;
        /// An AA tower this close already covers it.
        ///
        /// Default: `200`.
        aa_cover_radius: Fixed = Fixed::from_raw(2000000), true;
        /// Attackers within this radius of a hit tower are blamed.
        ///
        /// Default: `500`.
        attacker_radius: Fixed = Fixed::from_raw(5000000), true;
        /// Attack episodes from one direction before walling.
        ///
        /// Default: `2`.
        repeat_attacks: i32 = 2, true;
        /// Hits closer than this are one episode.
        ///
        /// Default: `20`.
        attack_gap_s: i32 = 20, true;
        /// Forget episodes after this.
        ///
        /// Default: `600`.
        attack_memory_s: i32 = 600, true;
        /// Extra constructors while walls are owed.
        ///
        /// Default: `1`.
        extra_constructors: i32 = 1, true;
    }
    /// Knobs named `mix.*`.
    mix: MixKnobs {
        /// Lab share: raider kbot.
        ///
        /// Default: `2`.
        lab_raider: i32 = 2, true;
        /// Lab share: rocket kbot.
        ///
        /// Default: `1`.
        lab_rocket: i32 = 1, true;
        /// Lab share: artillery kbot.
        ///
        /// Default: `0`.
        lab_artillery: i32 = 0, true;
        /// Vehicle plant share: light tank.
        ///
        /// Default: `1`.
        vp_light_tank: i32 = 1, true;
        /// Vehicle plant share: missile truck.
        ///
        /// Default: `0`.
        vp_missile: i32 = 0, true;
        /// Vehicle plant share: medium tank.
        ///
        /// Default: `0`.
        vp_medium_tank: i32 = 0, true;
        /// Lean the shares against the observed enemy.
        ///
        /// Default: `true`.
        counter: bool = true, true;
        /// Share points added by the static/mobile rule.
        ///
        /// Default: `2`.
        counter_bonus: i32 = 2, true;
        /// Fraction of the enemy army that triggers a lean.
        ///
        /// Default: `0.3`.
        counter_trigger: Fixed = Fixed::from_raw(3000), true;
        /// Keep a decaying role histogram of the enemy.
        ///
        /// Default: `true`.
        memory: bool = true, true;
        /// Halve the histogram this often.
        ///
        /// Default: `30`.
        memory_halflife_s: i32 = 30, true;
        /// Recompute the lean at most this often.
        ///
        /// Default: `60`.
        commit_s: i32 = 60, true;
        /// Share points a triggered role adds.
        ///
        /// Default: `2`.
        lean: i32 = 2, true;
    }
    /// Knobs named `tech.*`.
    tech: TechKnobs {
        /// Go to tier two when it pays.
        ///
        /// Default: `true`.
        enabled: bool = true, true;
        /// Metal income needed.
        ///
        /// Default: `10`.
        min_income: i32 = 10, true;
        /// Tier-two assault unit must be worth this much more per metal.
        ///
        /// Default: `1.5`.
        min_value_ratio: Fixed = Fixed::from_raw(15000), true;
    }
    /// Knobs named `air.*`.
    air: AirKnobs {
        /// Fighter floor.
        ///
        /// Default: `2`.
        fighters: i32 = 2, true;
        /// Fighter ceiling when answering enemy air.
        ///
        /// Default: `8`.
        fighters_max: i32 = 8, true;
        /// Bomber target.
        ///
        /// Default: `4`.
        bombers: i32 = 4, true;
        /// Gunship target.
        ///
        /// Default: `3`.
        gunships: i32 = 3, true;
        /// Scout plane target.
        ///
        /// Default: `1`.
        scout_planes: i32 = 1, true;
        /// Air transport target (not exposed to personality files).
        ///
        /// Default: `1`.
        transports: i32 = 1, false;
        /// Fighters on standing patrol.
        ///
        /// Default: `2`.
        patrol_fighters: i32 = 2, true;
        /// Bombers on standing patrol.
        ///
        /// Default: `2`.
        patrol_bombers: i32 = 2, true;
        /// Patrol arc distance past the base edge.
        ///
        /// Default: `400`.
        patrol_margin: Fixed = Fixed::from_raw(4000000), true;
        /// Idle aircraft further than this return.
        ///
        /// Default: `2400`.
        leash: Fixed = Fixed::from_raw(24000000), true;
        /// Enemy armed buildings that make air worth teching.
        ///
        /// Default: `4`.
        worth_it_defences: i32 = 4, true;
        /// Hold bombers and gunships for the ground wave.
        ///
        /// Default: `true`.
        sync: bool = true, true;
        /// Release radius.
        ///
        /// Default: `600`.
        sync_radius: Fixed = Fixed::from_raw(6000000), true;
        /// Bomb intruders within this radius of base first.
        ///
        /// Default: `900`.
        bomb_home_radius: Fixed = Fixed::from_raw(9000000), true;
        /// Skip targets under more AA than this.
        ///
        /// Default: `2`.
        bomb_max_aa: i32 = 2, true;
        /// Score multiplier for factories.
        ///
        /// Default: `2`.
        bomb_factory_weight: Fixed = Fixed::from_raw(20000), true;
        /// Score divisor per AA unit.
        ///
        /// Default: `1`.
        bomb_aa_penalty: Fixed = Fixed::from_raw(10000), true;
        /// Distance scale in the score.
        ///
        /// Default: `2400`.
        bomb_distance_scale: Fixed = Fixed::from_raw(24000000), true;
        /// Targets this close to the wave are discounted.
        ///
        /// Default: `700`.
        bomb_army_reach: Fixed = Fixed::from_raw(7000000), true;
        /// ...by this factor.
        ///
        /// Default: `0.25`.
        bomb_army_discount: Fixed = Fixed::from_raw(2500), true;
        /// Cluster radius for mobile targets.
        ///
        /// Default: `300`.
        bomb_cluster_radius: Fixed = Fixed::from_raw(3000000), true;
        /// Minimum cluster size.
        ///
        /// Default: `3`.
        bomb_cluster_min: i32 = 3, true;
        /// Gunships needed before they fly.
        ///
        /// Default: `2`.
        gunship_pack: i32 = 2, true;
        /// Gunships support the wave within this radius.
        ///
        /// Default: `1400`.
        gunship_radius: Fixed = Fixed::from_raw(14000000), true;
        /// Torpedo seaplane target.
        ///
        /// Default: `6`.
        torpedo_planes: i32 = 6, true;
    }
    /// Knobs named `scout.*`.
    scout: ScoutKnobs {
        /// 0 disables scouting (not exposed to personality files).
        ///
        /// Default: `1`.
        enabled: i32 = 1, false;
        /// Scout vehicle target.
        ///
        /// Default: `1`.
        vehicles: i32 = 1, true;
        /// Borrow a hull to scout when nothing enemy is known.
        ///
        /// Default: `true`.
        naval: bool = true, true;
    }
    /// Knobs named `army.*`.
    army: ArmyKnobs {
        /// Attack in waves (otherwise the whole army).
        ///
        /// Default: `true`.
        waves: bool = true, true;
        /// Army size that launches an attack.
        ///
        /// Default: `8`.
        wave_size: i32 = 8, true;
        /// A wave smaller than this is spent.
        ///
        /// Default: `3`.
        spent_size: i32 = 3, true;
        /// Join the wave in groups of at least this.
        ///
        /// Default: `3`.
        reinforce: i32 = 3, true;
        /// Attack anyway after this long without attacking.
        ///
        /// Default: `240`.
        patience_s: i32 = 240, true;
        /// Also attack when our army outvalues theirs.
        ///
        /// Default: `true`.
        odds: bool = true, true;
        /// Ratio that opens the odds gate.
        ///
        /// Default: `1.1`.
        odds_open: Fixed = Fixed::from_raw(11000), true;
        /// Ratio that closes it.
        ///
        /// Default: `0.7`.
        odds_close: Fixed = Fixed::from_raw(7000), true;
        /// Hold part of the wave back.
        ///
        /// Default: `true`.
        reserve: bool = true, true;
        /// Fraction committed at formation.
        ///
        /// Default: `0.7`.
        commit_frac: Fixed = Fixed::from_raw(7000), true;
        /// Release the reserve after this much contact.
        ///
        /// Default: `20`.
        reserve_release_s: i32 = 20, true;
        /// Waves use attack-move.
        ///
        /// Default: `true`.
        attack_move: bool = true, true;
        /// Leaders wait for the pack beyond this (0 = off).
        ///
        /// Default: `0`.
        cohesion: Fixed = Fixed::from_raw(0), true;
        /// Enemy units that divert the wave to fight them.
        ///
        /// Default: `3`.
        meet_count: i32 = 3, true;
        /// ...within this radius of the wave.
        ///
        /// Default: `900`.
        meet_radius: Fixed = Fixed::from_raw(9000000), true;
        /// Wave objectives are kept within this radius of the enemy base.
        ///
        /// Default: `900`.
        target_base_radius: Fixed = Fixed::from_raw(9000000), true;
        /// Weight of enemy firepower against value in objective scoring.
        ///
        /// Default: `1`.
        threat_aversion: Fixed = Fixed::from_raw(10000), true;
        /// Rally point distance from base towards the enemy.
        ///
        /// Default: `220`.
        rally: Fixed = Fixed::from_raw(2200000), true;
        /// Base acquisition radius.
        ///
        /// Default: `450`.
        engage: Fixed = Fixed::from_raw(4500000), true;
        /// Acquire at own weapon reach when longer.
        ///
        /// Default: `true`.
        acquire_at_range: bool = true, true;
        /// Back off from shorter-ranged mobile targets.
        ///
        /// Default: `true`.
        kite: bool = true, true;
        /// Margin in the kiting and acquisition rules.
        ///
        /// Default: `40`.
        kite_margin: Fixed = Fixed::from_raw(400000), true;
        /// Move when terrain blocks a direct shot.
        ///
        /// Default: `true`.
        blocked_shots: bool = true, true;
        /// Reposition when a target takes no damage.
        ///
        /// Default: `true`.
        stalls: bool = true, true;
        /// No-damage time before repositioning.
        ///
        /// Default: `15`.
        stall_s: i32 = 15, true;
        /// Sidestep distance.
        ///
        /// Default: `240`.
        stall_sidestep: Fixed = Fixed::from_raw(2400000), true;
        /// Repositions before giving up.
        ///
        /// Default: `2`.
        stall_tries: i32 = 2, true;
        /// Give up on the target for this long.
        ///
        /// Default: `60`.
        stall_forget_s: i32 = 60, true;
        /// Wave focuses one target (off).
        ///
        /// Default: `false`.
        focus_fire: bool = false, true;
        /// Focus target held this long.
        ///
        /// Default: `5`.
        focus_latch_s: i32 = 5, true;
        /// Fight one enemy player at a time.
        ///
        /// Default: `true`.
        one_enemy: bool = true, true;
        /// Switch enemy only if another is this much nearer.
        ///
        /// Default: `600`.
        enemy_switch_margin: Fixed = Fixed::from_raw(6000000), true;
        /// Send a raiding party while a wave is out.
        ///
        /// Default: `true`.
        raids: bool = true, true;
        /// Raiding party size.
        ///
        /// Default: `3`.
        raid_size: i32 = 3, true;
        /// Raids avoid this radius around the enemy base.
        ///
        /// Default: `900`.
        raid_avoid_base: Fixed = Fixed::from_raw(9000000), true;
        /// Hold the raid until the wave engages.
        ///
        /// Default: `true`.
        raid_sync: bool = true, true;
        /// ...for at most this long.
        ///
        /// Default: `15`.
        raid_sync_s: i32 = 15, true;
        /// Prefer fresh enemy expansion frames and their builders.
        ///
        /// Default: `true`.
        raid_frames: bool = true, true;
        /// A frame is fresh this long after first sight.
        ///
        /// Default: `30`.
        raid_frame_window_s: i32 = 30, true;
        /// Make the enemy commander the objective.
        ///
        /// Default: `true`.
        hunt_commander: bool = true, true;
        /// ...only when it is exposed.
        ///
        /// Default: `true`.
        hunt_exposed_only: bool = true, true;
        /// Exposed: this far from its base.
        ///
        /// Default: `600`.
        hunt_min_distance: Fixed = Fixed::from_raw(6000000), true;
        /// Escort counted within this radius.
        ///
        /// Default: `450`.
        hunt_escort_radius: Fixed = Fixed::from_raw(4500000), true;
        /// Exposed: escort worth less than this.
        ///
        /// Default: `600`.
        hunt_max_escort: i32 = 600, true;
        /// Exposure verdict held this long.
        ///
        /// Default: `10`.
        hunt_latch_s: i32 = 10, true;
        /// Enemies inside this radius of base are intruders.
        ///
        /// Default: `900`.
        defend_radius: Fixed = Fixed::from_raw(9000000), true;
        /// Do not charge intruders that outnumber us.
        ///
        /// Default: `true`.
        hold_outnumbered: bool = true, true;
        /// Units at home answer intruders while the wave stays out.
        ///
        /// Default: `true`.
        reserve_defends: bool = true, true;
        /// Send only enough to beat the intruders.
        ///
        /// Default: `true`.
        proportional: bool = true, true;
        /// Metal sent per intruder metal.
        ///
        /// Default: `1.3`.
        respond_ratio: Fixed = Fixed::from_raw(13000), true;
        /// Minimum responders.
        ///
        /// Default: `1`.
        respond_min: i32 = 1, true;
        /// Maximum share of home units sent.
        ///
        /// Default: `1`.
        respond_max_share: Fixed = Fixed::from_raw(10000), true;
        /// Recall the wave when base threat exceeds wave value by this.
        ///
        /// Default: `1.5`.
        recall_ratio: Fixed = Fixed::from_raw(15000), true;
        /// Keep the 16-sector threat memory.
        ///
        /// Default: `true`.
        sectors: bool = true, true;
        /// Halve sector threat this often.
        ///
        /// Default: `60`.
        sector_halflife_s: i32 = 60, true;
        /// Radar warning ring, in multiples of the defend radius.
        ///
        /// Default: `2.5`.
        radar_rings: Fixed = Fixed::from_raw(25000), true;
        /// Closing contacts that raise the warning.
        ///
        /// Default: `2`.
        radar_min_contacts: i32 = 2, true;
        /// Answer raids on outlying buildings.
        ///
        /// Default: `true`.
        outpost_response: bool = true, true;
        /// Raider search radius at the outpost.
        ///
        /// Default: `350`.
        outpost_raid_radius: Fixed = Fixed::from_raw(3500000), true;
        /// Responders within this radius.
        ///
        /// Default: `1500`.
        outpost_response_radius: Fixed = Fixed::from_raw(15000000), true;
        /// Responder metal per raider metal.
        ///
        /// Default: `1.2`.
        outpost_strength: Fixed = Fixed::from_raw(12000), true;
        /// Use the engagement estimate instead (off).
        ///
        /// Default: `false`.
        outpost_predictor: bool = false, true;
        /// Estimate margin needed.
        ///
        /// Default: `0`.
        outpost_predictor_margin: Fixed = Fixed::from_raw(0), true;
        /// Send damaged units home to be mended.
        ///
        /// Default: `true`.
        retreat_damaged: bool = true, true;
        /// Raider retreat health percent.
        ///
        /// Default: `30`.
        retreat_raider_pct: i32 = 30, true;
        /// Other units' retreat health percent.
        ///
        /// Default: `45`.
        retreat_line_pct: i32 = 45, true;
        /// Rejoin above this health percent.
        ///
        /// Default: `60`.
        rejoin_pct: i32 = 60, true;
        /// Wait at most this long for mending.
        ///
        /// Default: `45`.
        mend_wait_s: i32 = 45, true;
        /// Only retreat if a builder exists.
        ///
        /// Default: `true`.
        mend_needs_mender: bool = true, true;
        /// Only retreat from within this distance.
        ///
        /// Default: `1800`.
        mend_max_walk: Fixed = Fixed::from_raw(18000000), true;
        /// Mend spots ring radius.
        ///
        /// Default: `160`.
        mend_spot_radius: Fixed = Fixed::from_raw(1600000), true;
        /// Move only if further than this from the spot.
        ///
        /// Default: `400`.
        mend_spot_tolerance: Fixed = Fixed::from_raw(4000000), true;
    }
    /// Knobs named `cmd.*`.
    cmd: CmdKnobs {
        /// Commander fights small threats.
        ///
        /// Default: `true`.
        stands_ground: bool = true, true;
        /// ...worth up to this; also its value as builder cover.
        ///
        /// Default: `600`.
        fight_max_metal: i32 = 600, true;
        /// ...while above this health percent.
        ///
        /// Default: `50`.
        retreat_pct: i32 = 50, true;
        /// Threat radius round the commander.
        ///
        /// Default: `450`.
        danger_radius: Fixed = Fixed::from_raw(4500000), true;
        /// Army units within this radius defend it.
        ///
        /// Default: `1500`.
        guard_radius: Fixed = Fixed::from_raw(15000000), true;
        /// Use the commander's special weapon.
        ///
        /// Default: `true`.
        dgun: bool = true, true;
        /// Aim it at the most valuable target.
        ///
        /// Default: `true`.
        dgun_by_value: bool = true, true;
        /// Do not abandon a frame it would lose.
        ///
        /// Default: `true`.
        keeps_frames: bool = true, true;
        /// Frame life below which it stays.
        ///
        /// Default: `20`.
        frame_absence_s: i32 = 20, true;
        /// Own cover counted within this radius.
        ///
        /// Default: `800`.
        frame_cover_radius: Fixed = Fixed::from_raw(8000000), true;
        /// Hand the frame to a builder within this radius.
        ///
        /// Default: `800`.
        frame_handover_radius: Fixed = Fixed::from_raw(8000000), true;
        /// Flee rather than fight ships.
        ///
        /// Default: `true`.
        shuns_ships: bool = true, true;
        /// Commander attacks factory harassers when there is no army (off).
        ///
        /// Default: `false`.
        answers_harassment: bool = false, true;
        /// Commander alone fights up to this many intruders.
        ///
        /// Default: `1`.
        defends_alone_max: i32 = 1, true;
        /// Plan the commander's sites on its own movement class.
        ///
        /// Default: `true`.
        own_reachability: bool = true, true;
    }
    /// Knobs named `navy.*`.
    navy: NavyKnobs {
        /// Warship target on water maps (half on mixed maps).
        ///
        /// Default: `6`.
        fleet: i32 = 6, true;
        /// Shipyard target.
        ///
        /// Default: `2`.
        shipyards: i32 = 2, true;
        /// Allow the shipyard early in the list.
        ///
        /// Default: `true`.
        early_shipyard: bool = true, true;
        /// Construction ships use the base list (off).
        ///
        /// Default: `false`.
        ship_builders_plan_base: bool = false, true;
        /// Minimum spacing between our yards.
        ///
        /// Default: `448`.
        yard_spacing: Fixed = Fixed::from_raw(4480000), true;
        /// Scout ship target.
        ///
        /// Default: `1`.
        scout_ships: i32 = 1, true;
        /// Sea transport target.
        ///
        /// Default: `1`.
        sea_transports: i32 = 1, true;
        /// Construction ship target.
        ///
        /// Default: `2`.
        construction_ships: i32 = 2, true;
        /// Submarine target.
        ///
        /// Default: `2`.
        subs: i32 = 2, true;
        /// Destroyers needed before submarines.
        ///
        /// Default: `3`.
        subs_after_destroyers: i32 = 3, true;
        /// Advanced shipyard target.
        ///
        /// Default: `1`.
        adv_yards: i32 = 1, true;
        /// Income for the advanced shipyard.
        ///
        /// Default: `15`.
        tech_income: i32 = 15, true;
        /// Cruiser target.
        ///
        /// Default: `4`.
        cruisers: i32 = 4, true;
        /// Battleship target.
        ///
        /// Default: `2`.
        battleships: i32 = 2, true;
        /// AA ship target.
        ///
        /// Default: `2`.
        aa_ships: i32 = 2, true;
        /// Seaplane plant target.
        ///
        /// Default: `1`.
        seaplane_plants: i32 = 1, true;
        /// Sonar target.
        ///
        /// Default: `1`.
        sonar: i32 = 1, true;
        /// Torpedo launcher target.
        ///
        /// Default: `1`.
        torpedo_launchers: i32 = 1, true;
        /// Hulls gathered before the fleet sails.
        ///
        /// Default: `3`.
        sail_size: i32 = 3, true;
        /// Fleet size that also triggers the land attack (0 = off).
        ///
        /// Default: `0`.
        attack_size: i32 = 0, true;
        /// Fleet station distance from the yard.
        ///
        /// Default: `320`.
        rally: Fixed = Fixed::from_raw(3200000), true;
        /// Naval no-damage time before repositioning.
        ///
        /// Default: `12`.
        stall_s: i32 = 12, true;
        /// Outranging hulls hold off.
        ///
        /// Default: `true`.
        stand_off: bool = true, true;
    }
    /// Knobs named `ferry.*`.
    ferry: FerryKnobs {
        /// Prioritise the first transport when the enemy is across water.
        ///
        /// Default: `true`.
        urgent: bool = true, true;
        /// After this time the transport outranks the scout too.
        ///
        /// Default: `480`.
        first_by_s: i32 = 480, true;
        /// Army size before a sea ferry sails.
        ///
        /// Default: `6`.
        min_party: i32 = 6, true;
        /// Decide ferry need from the start positions and remember it.
        ///
        /// Default: `true`.
        want_from_map: bool = true, true;
        /// Gather passengers on a shore point.
        ///
        /// Default: `true`.
        muster: bool = true, true;
        /// Search landing points on a fan of bearings.
        ///
        /// Default: `true`.
        landing_fan: bool = true, true;
        /// Score landing points by threat.
        ///
        /// Default: `true`.
        landing_avoid_threat: bool = true, true;
        /// Threat radius for that score.
        ///
        /// Default: `300`.
        landing_threat_radius: Fixed = Fixed::from_raw(3000000), true;
        /// Landing search rings.
        ///
        /// Default: `24`.
        landing_rings: i32 = 24, true;
        /// Grid size for remembering failed landings.
        ///
        /// Default: `192`.
        refusal_cell: i32 = 192, true;
        /// Remember a failed landing this long.
        ///
        /// Default: `30`.
        refusal_s: i32 = 30, true;
        /// A loaded hull making no progress this long re-plans.
        ///
        /// Default: `20`.
        stall_s: i32 = 20, true;
        /// Abandon a pickup after this long (not exposed to personality files).
        ///
        /// Default: `120`.
        pickup_timeout_s: i32 = 120, false;
    }
    /// Knobs named `misc.*`.
    misc: MiscKnobs {
        /// The seat does nothing (difficulty "idle").
        ///
        /// Default: `false`.
        idle: bool = false, true;
    }
}
