use crate::agent::{Agent, MERCHANT_CARGO, MERCHANT_DISPATCH_THRESHOLD, MERCHANT_LOAD_MIN};
use crate::chronicle::{Chronicle, Event, TICKS_PER_YEAR};
use crate::world::{Biome, World};
use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::HashMap;

/// Trips on a declared route beyond which the two settlements pledge alliance.
const ALLIANCE_TRIPS: u32 = 5;
/// Maximum hex distance between settlements for a raid to be launched.
const RAID_MAX_DISTANCE: i32 = 12;
/// Stockpile below which a settlement becomes hungry enough to consider raiding.
const RAID_HUNGER_STOCK: f32 = 14.0;
/// Stockpile below which a settlement is outright starving — raids much more often.
const RAID_FAMINE_STOCK: f32 = 5.0;
/// Target must hold at least this much to be worth raiding.
const RAID_TARGET_STOCK: f32 = 25.0;
/// A raider settlement must muster this many warriors to attempt a raid.
const RAID_MIN_WARRIORS: u32 = 1;
/// Per-tick chance of rolling for a raid when conditions are met.
const RAID_CHANCE: f64 = 0.050;
/// Multiplier applied to raid chance when a settlement is in famine.
const RAID_FAMINE_MULT: f64 = 2.5;
/// Raids accumulated before a blood feud is declared.
const BLOOD_FEUD_THRESHOLD: u32 = 2;
/// A settlement can't be a raider until it has existed for this many ticks.
/// Prevents fresh clusters from immediately warring once a handful of their
/// founders happen to be veterans.
const RAID_MIN_SETTLEMENT_AGE_TICKS: u64 = 2 * TICKS_PER_YEAR;

/// Minimum loyal / nearby agents required to found a settlement.
const FOUND_THRESHOLD: usize = 5;
/// Radius (in hexes) within which agents count as "together".
const CLUSTER_RADIUS: i32 = 2;
/// Don't found a new settlement within this many hexes of an existing one.
const MIN_SEPARATION: i32 = 6;

#[derive(Debug, Clone)]
pub struct Route {
    pub other_id: u32,
    pub trips: u32,
    pub declared: bool,
    pub allied: bool,
}

#[derive(Debug, Clone)]
pub struct Enmity {
    pub other_id: u32,
    pub raids: u32,
    pub blood_feud: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trait {
    Militant,
    Mercantile,
}

const TRAIT_RAIDS_THRESHOLD: u32 = 3;
const TRAIT_TRADES_THRESHOLD: u32 = 8;

/// Per-tick probability that any single eligible custom emerges for a
/// settlement. ~1% keeps emergence occasional and gives multiple candidates
/// time to compete — a settlement with two eligible customs averages one per
/// hundred ticks, roughly a year.
const CUSTOM_CHANCE_PER_TICK: f64 = 0.010;
/// Minimum settlement age before any custom can emerge — a tradition needs
/// enough lived years behind it to feel earned.
const CUSTOM_MIN_AGE_TICKS: u64 = 2 * TICKS_PER_YEAR;
const CUSTOM_HARVEST_OVERFLOWS: u32 = 2;
const CUSTOM_WARRIOR_RAIDS: u32 = 3;
const CUSTOM_MEMORIAL_SUFFERED: u32 = 2;
const CUSTOM_MERCHANT_TRADES: u32 = 8;
/// Hexes within which a mountain tile counts as "nearby" for pilgrimage.
const CUSTOM_MOUNTAIN_RADIUS: i32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomKind {
    HarvestFeast,
    WarriorRite,
    MemorialVigil,
    MerchantFair,
    RiverBlessing,
    MountainPilgrimage,
}

const ALL_CUSTOM_KINDS: [CustomKind; 6] = [
    CustomKind::HarvestFeast,
    CustomKind::WarriorRite,
    CustomKind::MemorialVigil,
    CustomKind::MerchantFair,
    CustomKind::RiverBlessing,
    CustomKind::MountainPilgrimage,
];

/// A cultural tradition a settlement has grown into over time. Each settlement
/// may develop several customs, at most one of each [`CustomKind`].
#[derive(Debug, Clone)]
pub struct Custom {
    pub kind: CustomKind,
    pub name: String,
    pub founded_tick: u64,
}

#[derive(Debug, Clone)]
pub struct Settlement {
    pub id: u32,
    pub name: String,
    pub col: i32,
    pub row: i32,
    pub founded_tick: u64,
    pub population: u32,
    pub alive: bool,
    pub stockpile: f32,
    pub overflow_declared: bool,
    pub routes: Vec<Route>,
    pub enmities: Vec<Enmity>,
    pub raids_done: u32,
    pub raids_suffered: u32,
    pub trades_completed: u32,
    pub population_peak: u32,
    pub trait_kind: Option<Trait>,
    pub legend_fifty: bool,
    pub legend_crash: bool,
    /// Current land-health state around the settlement.
    pub land_depleted: bool,
    /// Year of the last depletion-or-recovery chronicle emission, for spam control.
    pub last_land_event_year: Option<u64>,
    /// Count of autumns in which the granary overflowed — a running tally of
    /// prosperous years used as a pressure source for harvest-feast customs.
    pub autumn_overflows: u32,
    /// Cultural traditions that have emerged from this settlement's behavior.
    pub customs: Vec<Custom>,
    /// Index into [`Dialects::centers`] for the language this settlement was
    /// named in. `None` when the world was generated without any centers
    /// (tiny maps, or [`Settlements::new`] used directly in tests).
    pub dialect_id: Option<u32>,
}

/// Signals emitted when a trade trip is recorded.
#[derive(Debug, Clone, Copy, Default)]
pub struct TripSignal {
    pub road_formed: bool,
    pub alliance_formed: bool,
}

impl Settlement {
    /// Check for a newly-emerged cultural trait; returns the chronicle line if one appeared.
    pub fn maybe_emerge_trait(&mut self) -> Option<String> {
        if self.trait_kind.is_some() {
            return None;
        }
        if self.raids_done >= TRAIT_RAIDS_THRESHOLD {
            self.trait_kind = Some(Trait::Militant);
            return Some(format!(
                "{} earns a reputation as a warlike people.",
                self.name
            ));
        }
        if self.trades_completed >= TRAIT_TRADES_THRESHOLD {
            self.trait_kind = Some(Trait::Mercantile);
            return Some(format!(
                "{} becomes known as a haven of trade.",
                self.name
            ));
        }
        None
    }

    pub fn note_trip(&mut self, other: u32) -> TripSignal {
        self.trades_completed += 1;
        let mut sig = TripSignal::default();
        for r in self.routes.iter_mut() {
            if r.other_id == other {
                r.trips += 1;
                if !r.declared && r.trips >= 3 {
                    r.declared = true;
                    sig.road_formed = true;
                }
                if !r.allied && r.trips >= ALLIANCE_TRIPS {
                    r.allied = true;
                    sig.alliance_formed = true;
                }
                return sig;
            }
        }
        self.routes.push(Route {
            other_id: other,
            trips: 1,
            declared: false,
            allied: false,
        });
        sig
    }

    /// True if this settlement already practices a custom of the given kind.
    pub fn has_custom(&self, kind: CustomKind) -> bool {
        self.customs.iter().any(|c| c.kind == kind)
    }

    /// Does the settlement currently meet the conditions for a custom of
    /// `kind`? Terrain-shaped customs consult the world map; the rest fall
    /// back to per-settlement behavioral counters.
    fn qualifies_for_custom(&self, kind: CustomKind, world: &World) -> bool {
        match kind {
            CustomKind::HarvestFeast => self.autumn_overflows >= CUSTOM_HARVEST_OVERFLOWS,
            CustomKind::WarriorRite => self.raids_done >= CUSTOM_WARRIOR_RAIDS,
            CustomKind::MemorialVigil => self.raids_suffered >= CUSTOM_MEMORIAL_SUFFERED,
            CustomKind::MerchantFair => self.trades_completed >= CUSTOM_MERCHANT_TRADES,
            CustomKind::RiverBlessing => {
                world.is_near_river(self.col, self.row)
                    || world.tile(self.col, self.row).map(|t| t.biome) == Some(Biome::Coast)
            }
            CustomKind::MountainPilgrimage => near_mountain(world, self.col, self.row),
        }
    }

    /// Per-tick dice roll for cultural emergence. Returns a chronicle line if
    /// a new custom took root. Each settlement can grow at most one custom of
    /// a given kind over its lifetime, but multiple different kinds can stack.
    pub fn maybe_emerge_custom(
        &mut self,
        world: &World,
        rng: &mut ChaCha8Rng,
        tick: u64,
    ) -> Option<String> {
        if tick.saturating_sub(self.founded_tick) < CUSTOM_MIN_AGE_TICKS {
            return None;
        }
        for &kind in ALL_CUSTOM_KINDS.iter() {
            if self.has_custom(kind) {
                continue;
            }
            if !self.qualifies_for_custom(kind, world) {
                continue;
            }
            if !rng.gen_bool(CUSTOM_CHANCE_PER_TICK) {
                continue;
            }
            let name = pick_custom_name(kind, rng);
            let line = custom_emergence_line(&self.name, kind, &name);
            self.customs.push(Custom {
                kind,
                name,
                founded_tick: tick,
            });
            return Some(line);
        }
        None
    }

    /// Record a raid against `other`; returns true if a blood feud was just declared.
    pub fn note_raid(&mut self, other: u32) -> bool {
        for e in self.enmities.iter_mut() {
            if e.other_id == other {
                e.raids += 1;
                if !e.blood_feud && e.raids >= BLOOD_FEUD_THRESHOLD {
                    e.blood_feud = true;
                    return true;
                }
                return false;
            }
        }
        self.enmities.push(Enmity {
            other_id: other,
            raids: 1,
            blood_feud: false,
        });
        false
    }
}

pub struct Settlements {
    pub list: Vec<Settlement>,
    next_id: u32,
    pub dialects: Dialects,
}

impl Settlements {
    pub fn new() -> Self {
        Self {
            list: Vec::new(),
            next_id: 0,
            dialects: Dialects::empty(),
        }
    }

    /// Replace this settlement book's dialects. Called once at sim startup,
    /// after the world is generated but before any settlement is founded.
    pub fn set_dialects(&mut self, dialects: Dialects) {
        self.dialects = dialects;
    }

    pub fn alive_count(&self) -> usize {
        self.list.iter().filter(|s| s.alive).count()
    }

    fn too_close(&self, world: &World, col: i32, row: i32) -> bool {
        self.list
            .iter()
            .filter(|s| s.alive)
            .any(|s| world.hex_distance((s.col, s.row), (col, row)) < MIN_SEPARATION)
    }

    fn found(&mut self, col: i32, row: i32, tick: u64, world: &World, rng: &mut ChaCha8Rng) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        let dialect_idx = self.dialects.nearest(world, col, row);
        let name = match dialect_idx {
            Some(i) => generate_name(rng, Some(&self.dialects.centers[i].dialect)),
            None => generate_name(rng, None),
        };
        self.list.push(Settlement {
            id,
            name,
            col,
            row,
            founded_tick: tick,
            population: 0,
            alive: true,
            stockpile: 0.0,
            overflow_declared: false,
            routes: Vec::new(),
            enmities: Vec::new(),
            raids_done: 0,
            raids_suffered: 0,
            trades_completed: 0,
            population_peak: 0,
            trait_kind: None,
            legend_fifty: false,
            legend_crash: false,
            land_depleted: false,
            last_land_event_year: None,
            autumn_overflows: 0,
            customs: Vec::new(),
            dialect_id: dialect_idx.map(|i| i as u32),
        });
        id
    }
}

/// Check each unattached agent for a cluster of neighbors; if one exists and no
/// settlement is already close by, found a new one. Also recomputes population
/// for existing settlements and abandons empty ones.
pub fn update_settlements(
    settlements: &mut Settlements,
    agents: &mut [Agent],
    world: &World,
    rng: &mut ChaCha8Rng,
    chronicle: &mut Chronicle,
    tick: u64,
) {
    // First, try to found new settlements around clusters of unaffiliated agents.
    let unaffiliated: Vec<usize> = agents
        .iter()
        .enumerate()
        .filter(|(_, a)| a.alive && !a.is_traveling() && a.settlement.is_none())
        .map(|(i, _)| i)
        .collect();

    // Gather every eligible cluster (size + not too close to existing settlements),
    // then found only the largest one this tick. Deliberate pacing.
    //
    // Spatial hash: bucket eligible agents by tile once (O(n)), then per-agent
    // only scan the hex neighborhood within CLUSTER_RADIUS instead of every
    // agent in the world.
    let mut position_bucket: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for &i in &unaffiliated {
        let a = &agents[i];
        position_bucket.entry((a.col, a.row)).or_default().push(i);
    }

    let mut best: Option<(usize, i32, i32, Vec<usize>)> = None;
    for &i in &unaffiliated {
        let (ac, ar) = (agents[i].col, agents[i].row);
        let mut neighbors: Vec<usize> = Vec::new();
        for dc in -CLUSTER_RADIUS..=CLUSTER_RADIUS {
            for dr in -CLUSTER_RADIUS..=CLUSTER_RADIUS {
                let c = ac + dc;
                let r = ar + dr;
                if world.hex_distance((ac, ar), (c, r)) > CLUSTER_RADIUS {
                    continue;
                }
                if let Some(list) = position_bucket.get(&(c, r)) {
                    neighbors.extend_from_slice(list);
                }
            }
        }
        if neighbors.len() >= FOUND_THRESHOLD && !settlements.too_close(world, ac, ar) {
            let better = match &best {
                None => true,
                Some((n, _, _, _)) => neighbors.len() > *n,
            };
            if better {
                best = Some((neighbors.len(), ac, ar, neighbors));
            }
        }
    }

    if let Some((_n, ac, ar, neighbors)) = best {
        let nearest = settlements
            .list
            .iter()
            .filter(|s| s.alive)
            .map(|s| (s.name.clone(), world.hex_distance((s.col, s.row), (ac, ar))))
            .min_by_key(|(_, d)| *d);

        let id = settlements.found(ac, ar, tick, world, rng);
        for &j in &neighbors {
            agents[j].settlement = Some(id);
            agents[j].deeds.founded_settlement = true;
            // No coin-flip role assignment — roles emerge from what an agent
            // does, not from a die roll at settlement-joining.
        }
        let s = settlements
            .list
            .iter()
            .find(|s| s.id == id)
            .expect("just pushed");
        let locator = match nearest {
            Some((other_name, d)) if d <= 15 => {
                let days = describe_distance(d);
                format!(" {} from {}", days, other_name)
            }
            _ => {
                if let Some(region) = world.region_at(s.col, s.row) {
                    format!(" in {}", region.name)
                } else {
                    let biome = world
                        .tile(s.col, s.row)
                        .map(|t| t.biome.name())
                        .unwrap_or("unknown land");
                    format!(" upon the {}", biome)
                }
            }
        };
        chronicle.record(Event::new(
            tick,
            format!(
                "A band of {} settlers gathers{}. They name the place {}.",
                neighbors.len(),
                locator,
                s.name
            ),
        ));
    }

    // Land-health chronicling: warn on depletion, celebrate recovery.
    update_land_health(settlements, world, chronicle, tick);

    // Build settlement -> agent-indices map once, to replace the O(s*n) per-sub-phase
    // scans below. Within this tick's remaining work, an agent's settlement field
    // only transitions Some(X)->None (via raid sack scatter or starvation
    // migration), so stale entries are filtered out by re-checking state at use.
    let members_by_settlement = build_members_map(agents);

    // Raids between settlements.
    raid_phase(settlements, agents, &members_by_settlement, world, rng, chronicle, tick);

    // Trade dispatch: settlements with surplus pick a skilled trader to send.
    try_dispatch_merchants(settlements, agents, &members_by_settlement, rng);

    // Migration: if a settlement's people are starving, some depart to wander.
    migrate_from_starving(
        settlements,
        agents,
        &members_by_settlement,
        world,
        rng,
        chronicle,
        tick,
    );

    // Granary overflow chronicling — only during Autumn (season index 2).
    let season_idx = (tick % TICKS_PER_YEAR) / (TICKS_PER_YEAR / 4);
    for s in settlements.list.iter_mut() {
        if !s.alive {
            continue;
        }
        if s.stockpile > 40.0 && !s.overflow_declared {
            if season_idx == 2 {
                s.overflow_declared = true;
                s.autumn_overflows += 1;
                chronicle.record(Event::new(
                    tick,
                    format!("The granary of {} overflows with autumn harvest.", s.name),
                ));
            }
        } else if s.stockpile < 20.0 && s.overflow_declared {
            s.overflow_declared = false;
        }
    }

    // Cultural customs emerge from accumulated behavior. Each alive settlement
    // rolls per tick against its candidate customs — the `maybe_emerge_custom`
    // call gates on age, qualification, and a small per-tick probability so
    // traditions take years to appear rather than minutes.
    let custom_lines: Vec<String> = settlements
        .list
        .iter_mut()
        .filter(|s| s.alive)
        .filter_map(|s| s.maybe_emerge_custom(world, rng, tick))
        .collect();
    for line in custom_lines {
        chronicle.record(Event::new(tick, line));
    }

    // Recount populations and retire any settlement that's lost all its people.
    // Build the count map in one O(n) pass so the per-settlement loop is O(1) each.
    let mut pop_by_settlement: HashMap<u32, u32> = HashMap::new();
    for a in agents.iter() {
        if a.alive {
            if let Some(sid) = a.settlement {
                *pop_by_settlement.entry(sid).or_insert(0) += 1;
            }
        }
    }
    for s in settlements.list.iter_mut() {
        if !s.alive {
            continue;
        }
        let pop = pop_by_settlement.get(&s.id).copied().unwrap_or(0);
        if pop == 0 && s.population > 0 {
            s.alive = false;
            chronicle.record(Event::new(
                tick,
                format!("{} is abandoned. The wind moves through empty halls.", s.name),
            ));
        }
        if pop > s.population_peak {
            s.population_peak = pop;
        }
        if !s.legend_fifty && pop >= 50 {
            s.legend_fifty = true;
            chronicle.record(Event::new(
                tick,
                format!("*** {} swells beyond fifty souls ***", s.name),
            ));
        }
        if !s.legend_crash && s.population_peak >= 20 && pop > 0 && pop < 10 {
            s.legend_crash = true;
            chronicle.record(Event::new(
                tick,
                format!("*** {} withers to a handful ***", s.name),
            ));
        }
        s.population = pop;
    }

    // Orphan agents whose settlement died. Walk only each dead settlement's
    // former members via the map instead of scanning every agent.
    for s in settlements.list.iter() {
        if s.alive {
            continue;
        }
        if let Some(list) = members_by_settlement.get(&s.id) {
            for &i in list {
                if agents[i].settlement == Some(s.id) {
                    agents[i].settlement = None;
                }
            }
        }
    }

    // Great individuals: recognize agents whose deeds cross notability thresholds.
    recognize_great_individuals(agents, chronicle, tick);
}

/// Scan all living agents and recognize any who have crossed a notability
/// threshold but haven't been given an epithet yet. Each agent earns at most
/// one epithet in their lifetime. The chronicle line includes their name with
/// the epithet and a brief description of what earned it.
fn recognize_great_individuals(
    agents: &mut [Agent],
    chronicle: &mut Chronicle,
    tick: u64,
) {
    use crate::agent::choose_epithet;
    for agent in agents.iter_mut() {
        if !agent.alive || agent.epithet.is_some() {
            continue;
        }
        if !agent.deeds.is_notable() {
            continue;
        }
        let epithet = choose_epithet(&agent.deeds, agent.id);
        let reason = describe_deeds(&agent.deeds);
        agent.epithet = Some(epithet.to_string());
        chronicle.record(Event::new(
            tick,
            format!(
                "*** {} {} — {} ***",
                agent.name, epithet, reason
            ),
        ));
    }
}

/// Produce a human-readable summary of what makes this agent notable.
fn describe_deeds(deeds: &crate::agent::Deeds) -> &'static str {
    if deeds.raids_led >= 2 {
        return "who led raids against rival settlements";
    }
    if deeds.deliveries >= 3 {
        return "who carried trade across the land";
    }
    if deeds.survived_sack {
        return "who survived the fall of their homeland";
    }
    if deeds.founded_settlement && deeds.defenses >= 2 {
        return "who founded a settlement and defended it twice";
    }
    "whose deeds will not be forgotten"
}

fn build_members_map(agents: &[Agent]) -> HashMap<u32, Vec<usize>> {
    let mut m: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, a) in agents.iter().enumerate() {
        if let Some(sid) = a.settlement {
            m.entry(sid).or_default().push(i);
        }
    }
    m
}

/// Rough human-scale distance description.
fn describe_distance(hexes: i32) -> &'static str {
    match hexes {
        0..=2 => "just a short walk",
        3..=5 => "a half-day's walk",
        6..=9 => "a day's walk",
        10..=15 => "several days' walk",
        _ => "a long journey",
    }
}

/// Members of settlements whose average hunger is high peel off and wander as unaffiliated.
fn migrate_from_starving(
    settlements: &mut Settlements,
    agents: &mut [Agent],
    members_by_settlement: &HashMap<u32, Vec<usize>>,
    world: &World,
    rng: &mut ChaCha8Rng,
    chronicle: &mut Chronicle,
    tick: u64,
) {
    const STARVE_AVG_HUNGER: f32 = 60.0;
    const MIN_POP_TO_MIGRATE: u32 = 4;

    // Gather per-settlement avg hunger & member indices.
    let alive_ids: Vec<u32> = settlements
        .list
        .iter()
        .filter(|s| s.alive && s.population >= MIN_POP_TO_MIGRATE)
        .map(|s| s.id)
        .collect();

    for sid in alive_ids {
        let members: Vec<usize> = members_by_settlement
            .get(&sid)
            .map(|list| {
                list.iter()
                    .copied()
                    .filter(|&i| {
                        let a = &agents[i];
                        a.alive && !a.is_traveling() && a.settlement == Some(sid)
                    })
                    .collect()
            })
            .unwrap_or_default();
        if members.len() < MIN_POP_TO_MIGRATE as usize {
            continue;
        }
        let avg_hunger: f32 =
            members.iter().map(|&i| agents[i].hunger).sum::<f32>() / members.len() as f32;
        if avg_hunger < STARVE_AVG_HUNGER {
            continue;
        }
        // ~25% of the hungriest depart.
        let mut by_hunger: Vec<usize> = members.clone();
        by_hunger.sort_by(|&a, &b| agents[b].hunger.partial_cmp(&agents[a].hunger).unwrap());
        let departing = (by_hunger.len() / 4).max(1);
        let leavers: Vec<usize> = by_hunger.into_iter().take(departing).collect();

        let sname = settlements
            .list
            .iter()
            .find(|s| s.id == sid)
            .map(|s| s.name.clone())
            .unwrap_or_default();

        for &i in &leavers {
            agents[i].settlement = None;
            // Nudge them to a passable neighbor so they begin drifting.
            let neigh: Vec<(i32, i32)> = world
                .neighbors(agents[i].col, agents[i].row)
                .into_iter()
                .filter(|&(c, r)| world.is_land(c, r))
                .collect();
            if !neigh.is_empty() {
                let (nc, nr) = neigh[rng.gen_range(0..neigh.len())];
                agents[i].col = nc;
                agents[i].row = nr;
            }
        }

        let n = leavers.len();
        let noun = if n == 1 { "soul departs" } else { "souls depart" };
        chronicle.record(Event::new(
            tick,
            format!("{} {} the starving halls of {}.", n, noun, sname),
        ));
    }
}

/// Each settlement with a surplus dispatches its most-skilled trader (if any
/// crosses `MERCHANT_DISPATCH_THRESHOLD`) on a one-trip delivery to a random
/// other living settlement. The candidate must be at home, loyal, and not
/// already on the road. Settlements without a skilled-enough trader simply
/// don't trade — early in a sim, that's most of them.
fn try_dispatch_merchants(
    settlements: &mut Settlements,
    agents: &mut [Agent],
    members_by_settlement: &HashMap<u32, Vec<usize>>,
    rng: &mut ChaCha8Rng,
) {
    let surplus_ids: Vec<u32> = settlements
        .list
        .iter()
        .filter(|s| s.alive && s.stockpile >= MERCHANT_LOAD_MIN)
        .map(|s| s.id)
        .collect();

    for sid in surplus_ids {
        let (hc, hr, stock) = match settlements.list.iter().find(|s| s.id == sid) {
            Some(s) => (s.col, s.row, s.stockpile),
            None => continue,
        };
        if stock < MERCHANT_LOAD_MIN {
            continue;
        }

        // Pick the loyal, at-home, idle agent with the highest trading skill,
        // gated at MERCHANT_DISPATCH_THRESHOLD. Ties broken by lowest id for
        // determinism.
        let mut best: Option<(usize, f32)> = None;
        if let Some(member_list) = members_by_settlement.get(&sid) {
            for &i in member_list {
                let a = &agents[i];
                if !a.alive
                    || a.settlement != Some(sid)
                    || a.is_traveling()
                    || (a.col, a.row) != (hc, hr)
                {
                    continue;
                }
                if a.skills.trading <= MERCHANT_DISPATCH_THRESHOLD {
                    continue;
                }
                match best {
                    Some((_, s)) if s >= a.skills.trading => {}
                    _ => best = Some((i, a.skills.trading)),
                }
            }
        }
        let Some((idx, _)) = best else { continue };

        let others: Vec<u32> = settlements
            .list
            .iter()
            .filter(|s| s.alive && s.id != sid)
            .map(|s| s.id)
            .collect();
        if others.is_empty() {
            continue;
        }
        let dest_id = others[rng.gen_range(0..others.len())];

        let load = MERCHANT_CARGO.min(stock);
        if let Some(s) = settlements.list.iter_mut().find(|s| s.id == sid) {
            s.stockpile -= load;
        }
        agents[idx].cargo = load;
        agents[idx].cargo_origin = Some(sid);
        agents[idx].destination = Some(dest_id);
    }
}

/// Count living warriors (fighting skill above the recognition threshold)
/// affiliated with settlement `sid`.
fn count_warriors(
    agents: &[Agent],
    members_by_settlement: &HashMap<u32, Vec<usize>>,
    sid: u32,
) -> u32 {
    let Some(list) = members_by_settlement.get(&sid) else {
        return 0;
    };
    list.iter()
        .filter(|&&i| {
            let a = &agents[i];
            a.alive && a.is_warrior() && a.settlement == Some(sid)
        })
        .count() as u32
}

/// Sum of fighting skill across all living warriors of settlement `sid`.
/// Used as the muster strength for raid resolution: a roomful of seasoned
/// warriors out-fights an equal count of journeymen.
fn warrior_strength(
    agents: &[Agent],
    members_by_settlement: &HashMap<u32, Vec<usize>>,
    sid: u32,
) -> f32 {
    let Some(list) = members_by_settlement.get(&sid) else {
        return 0.0;
    };
    list.iter()
        .filter_map(|&i| {
            let a = &agents[i];
            if a.alive && a.is_warrior() && a.settlement == Some(sid) {
                Some(a.skills.fighting)
            } else {
                None
            }
        })
        .sum()
}

/// Mark the single highest-fighting warrior of settlement `sid` as having
/// led a raid. If multiple are tied, the lowest-indexed agent wins (deterministic).
fn mark_raid_leader(
    agents: &mut [Agent],
    members_by_settlement: &HashMap<u32, Vec<usize>>,
    sid: u32,
) {
    let Some(list) = members_by_settlement.get(&sid) else {
        return;
    };
    let mut best: Option<usize> = None;
    let mut best_skill: f32 = -1.0;
    for &i in list {
        let a = &agents[i];
        if a.alive && a.is_warrior() && a.settlement == Some(sid) && a.skills.fighting > best_skill
        {
            best = Some(i);
            best_skill = a.skills.fighting;
        }
    }
    if let Some(idx) = best {
        agents[idx].deeds.raids_led += 1;
    }
}

/// Kill up to `n` warriors belonging to settlement `sid`, returning how many fell.
fn slay_warriors(
    agents: &mut [Agent],
    members_by_settlement: &HashMap<u32, Vec<usize>>,
    sid: u32,
    n: u32,
) -> u32 {
    let Some(list) = members_by_settlement.get(&sid) else {
        return 0;
    };
    let mut killed = 0u32;
    for &i in list {
        if killed >= n {
            break;
        }
        let a = &mut agents[i];
        if a.alive && a.is_warrior() && a.settlement == Some(sid) {
            a.alive = false;
            killed += 1;
        }
    }
    killed
}

/// Apply warrior casualties, chronicle a "Battle of X" line when losses are
/// heavy, record the raid on both sides, and chronicle any fresh blood feud.
/// Shared post-raid bookkeeping for the success and repelled branches (the
/// sack branch is distinct because the target is destroyed outright).
fn resolve_raid_outcome(
    settlements: &mut Settlements,
    agents: &mut [Agent],
    members_by_settlement: &HashMap<u32, Vec<usize>>,
    raider_id: u32,
    target_id: u32,
    raider_name: &str,
    target_name: &str,
    atk_losses: u32,
    def_losses: u32,
    chronicle: &mut Chronicle,
    tick: u64,
) {
    slay_warriors(agents, members_by_settlement, raider_id, atk_losses);
    slay_warriors(agents, members_by_settlement, target_id, def_losses);

    if atk_losses + def_losses >= 3 {
        chronicle.record(Event::new(
            tick,
            format!(
                "*** The Battle of {} — {} warriors fall ***",
                target_name,
                atk_losses + def_losses
            ),
        ));
    }

    let feud_r = settlements
        .list
        .iter_mut()
        .find(|s| s.id == raider_id)
        .map(|s| s.note_raid(target_id))
        .unwrap_or(false);
    let feud_t = settlements
        .list
        .iter_mut()
        .find(|s| s.id == target_id)
        .map(|s| s.note_raid(raider_id))
        .unwrap_or(false);
    if feud_r || feud_t {
        chronicle.record(Event::new(
            tick,
            format!(
                "A blood feud takes root between {} and {}.",
                raider_name, target_name
            ),
        ));
    }
}

/// Reward survivors of `sid` with combat experience after a raid, and emit a
/// one-per-year chronicle line for any who just crossed into "seasoned
/// warrior" territory. When `warriors_only` is true, only agents already past
/// the warrior threshold learn — that's correct for attackers (only warriors
/// rode out). When false, every living settlement member gains skill — used
/// for defenders, because being raided is itself a combat lesson and is how
/// non-warriors first rise above [`WARRIOR_RECOGNITION_THRESHOLD`].
fn grant_combat_experience(
    agents: &mut [Agent],
    members_by_settlement: &HashMap<u32, Vec<usize>>,
    sid: u32,
    warriors_only: bool,
    chronicle: &mut Chronicle,
    tick: u64,
) {
    use crate::agent::{FIGHTING_GROWTH, ROLE_RECOGNITION_THRESHOLD};
    let year = tick / crate::chronicle::TICKS_PER_YEAR;
    let Some(list) = members_by_settlement.get(&sid) else {
        return;
    };
    for &i in list {
        let a = &mut agents[i];
        if !a.alive || a.settlement != Some(sid) {
            continue;
        }
        if warriors_only && !a.is_warrior() {
            continue;
        }
        // Track defense deeds for the defending side (warriors_only == false).
        if !warriors_only && a.is_warrior() {
            a.deeds.defenses += 1;
        }
        let before = a.skills.fighting;
        a.skills.fighting = (a.skills.fighting + FIGHTING_GROWTH).min(1.0);
        if before < ROLE_RECOGNITION_THRESHOLD
            && a.skills.fighting >= ROLE_RECOGNITION_THRESHOLD
            && a.last_skill_event_year != Some(year)
        {
            a.last_skill_event_year = Some(year);
            chronicle.record(crate::chronicle::Event::new(
                tick,
                format!("{} has become a seasoned warrior.", a.name),
            ));
        }
    }
}

/// Hungry settlements with warriors may raid a nearby wealthy neighbor.
fn raid_phase(
    settlements: &mut Settlements,
    agents: &mut [Agent],
    members_by_settlement: &HashMap<u32, Vec<usize>>,
    world: &World,
    rng: &mut ChaCha8Rng,
    chronicle: &mut Chronicle,
    tick: u64,
) {
    // Snapshot candidate raiders to avoid borrow issues during resolution.
    // New settlements can't raid — a lowered warrior threshold means raw
    // clusters would otherwise war immediately if a few founders happened to
    // seed high on fighting.
    let candidates: Vec<(u32, i32, i32, f32)> = settlements
        .list
        .iter()
        .filter(|s| {
            s.alive
                && s.stockpile < RAID_HUNGER_STOCK
                && tick.saturating_sub(s.founded_tick) >= RAID_MIN_SETTLEMENT_AGE_TICKS
        })
        .map(|s| (s.id, s.col, s.row, s.stockpile))
        .collect();

    for (raider_id, rc, rr, _rstock) in candidates {
        // Re-check raider: prior iterations may have destroyed it, or filled its
        // stockpile via loot so it's no longer hungry enough to raid again.
        let current_stock = match settlements
            .list
            .iter()
            .find(|s| s.id == raider_id && s.alive)
        {
            Some(s) => s.stockpile,
            None => continue,
        };
        if current_stock >= RAID_HUNGER_STOCK {
            continue;
        }
        let _rstock = current_stock;
        let attackers = count_warriors(agents, members_by_settlement, raider_id);
        let attacker_strength = warrior_strength(agents, members_by_settlement, raider_id);
        if attackers < RAID_MIN_WARRIORS {
            continue;
        }
        // Compute allied list early so we can weight chance by proximity of non-allied neighbors.
        let (raider_allied, raider_enemies): (Vec<u32>, Vec<u32>) = settlements
            .list
            .iter()
            .find(|s| s.id == raider_id)
            .map(|s| {
                (
                    s.routes.iter().filter(|r| r.allied).map(|r| r.other_id).collect(),
                    s.enmities.iter().map(|e| e.other_id).collect(),
                )
            })
            .unwrap_or_default();

        // Proximity multiplier: close neighbors make raids more likely.
        let nearest_dist = settlements
            .list
            .iter()
            .filter(|s| {
                s.alive
                    && s.id != raider_id
                    && !raider_allied.contains(&s.id)
                    && world.hex_distance((s.col, s.row), (rc, rr)) <= RAID_MAX_DISTANCE
            })
            .map(|s| world.hex_distance((s.col, s.row), (rc, rr)))
            .min();
        let proximity_mult = match nearest_dist {
            Some(d) if d <= 4 => 3.0,
            Some(d) if d <= 7 => 1.8,
            _ => 1.0,
        };
        let base = if _rstock < RAID_FAMINE_STOCK {
            RAID_CHANCE * RAID_FAMINE_MULT
        } else {
            RAID_CHANCE
        };
        let chance = (base * proximity_mult).min(1.0);
        if !rng.gen_bool(chance) {
            continue;
        }

        let enemy_target = settlements
            .list
            .iter()
            .filter(|s| {
                s.alive
                    && s.id != raider_id
                    && raider_enemies.contains(&s.id)
                    && !raider_allied.contains(&s.id)
                    && world.hex_distance((s.col, s.row), (rc, rr)) <= RAID_MAX_DISTANCE
            })
            .max_by(|a, b| a.stockpile.partial_cmp(&b.stockpile).unwrap())
            .map(|s| s.id);

        let target_opt = enemy_target.or_else(|| {
            settlements
                .list
                .iter()
                .filter(|s| {
                    s.alive
                        && s.id != raider_id
                        && !raider_allied.contains(&s.id)
                        && s.stockpile >= RAID_TARGET_STOCK
                        && world.hex_distance((s.col, s.row), (rc, rr)) <= RAID_MAX_DISTANCE
                })
                .max_by(|a, b| a.stockpile.partial_cmp(&b.stockpile).unwrap())
                .map(|s| s.id)
        });

        let Some(target_id) = target_opt else { continue };

        let own_defenders = count_warriors(agents, members_by_settlement, target_id);
        let own_defender_strength = warrior_strength(agents, members_by_settlement, target_id);
        // Allies of the target pledge mutual defense — their warriors join the fight.
        let target_allies: Vec<u32> = settlements
            .list
            .iter()
            .find(|s| s.id == target_id)
            .map(|s| s.routes.iter().filter(|r| r.allied).map(|r| r.other_id).collect())
            .unwrap_or_default();
        let ally_defenders: u32 = target_allies
            .iter()
            .filter(|&&aid| settlements.list.iter().any(|s| s.id == aid && s.alive))
            .map(|&aid| count_warriors(agents, members_by_settlement, aid))
            .sum();
        let ally_defender_strength: f32 = target_allies
            .iter()
            .filter(|&&aid| settlements.list.iter().any(|s| s.id == aid && s.alive))
            .map(|&aid| warrior_strength(agents, members_by_settlement, aid))
            .sum();
        let defenders = own_defenders + ally_defenders;
        let defender_strength = own_defender_strength + ally_defender_strength;

        let raider_name = settlements
            .list
            .iter()
            .find(|s| s.id == raider_id)
            .map(|s| s.name.clone())
            .unwrap();
        let target_name = settlements
            .list
            .iter()
            .find(|s| s.id == target_id)
            .map(|s| s.name.clone())
            .unwrap();

        chronicle.record(Event::new(
            tick,
            format!(
                "Warriors of {} descend upon {} under cover of night.",
                raider_name, target_name
            ),
        ));

        // Resolve: skill-weighted muster + a die roll for chaos. A roomful
        // of seasoned warriors out-fights an equal count of journeymen.
        let atk_roll = attacker_strength + rng.gen_range(0.0..3.0);
        let def_roll = defender_strength + 1.0 + rng.gen_range(0.0..3.0);

        // Sack: attackers outnumber defenders 2x or more.
        let sack = attackers >= 3 && attackers >= defenders.saturating_mul(2);
        let success = atk_roll > def_roll;

        // Bump aggregate counters used for trait emergence.
        if let Some(r) = settlements.list.iter_mut().find(|s| s.id == raider_id) {
            r.raids_done += 1;
        }
        if let Some(t) = settlements.list.iter_mut().find(|s| s.id == target_id) {
            t.raids_suffered += 1;
        }

        if sack {
            // Mark the top warrior of the raider as having led this raid.
            mark_raid_leader(agents, members_by_settlement, raider_id);
            // Full sack: destroy defender, transfer stockpile, scatter civilians.
            let (loot, t_col, t_row) = {
                let t = settlements
                    .list
                    .iter_mut()
                    .find(|s| s.id == target_id)
                    .unwrap();
                let loot = t.stockpile;
                t.stockpile = 0.0;
                t.alive = false;
                (loot, t.col, t.row)
            };
            slay_warriors(agents, members_by_settlement, target_id, defenders);
            // Surviving civilians of target lose affiliation. Their skills
            // travel with them — wherever they end up, they remember.
            if let Some(list) = members_by_settlement.get(&target_id) {
                for &i in list {
                    let a = &mut agents[i];
                    if a.alive && a.settlement == Some(target_id) {
                        a.deeds.survived_sack = true;
                        a.settlement = None;
                        a.cargo = 0.0;
                        a.cargo_origin = None;
                        a.destination = None;
                    }
                }
            }
            // Attacker loses a couple of warriors even in victory.
            let atk_losses = rng.gen_range(0..=2).min(attackers.saturating_sub(1));
            slay_warriors(agents, members_by_settlement, raider_id, atk_losses);
            if let Some(r) = settlements.list.iter_mut().find(|s| s.id == raider_id) {
                r.stockpile += loot;
            }
            chronicle.record(Event::new(
                tick,
                format!(
                    "{} is put to the torch. The smoke rises above empty fields.",
                    target_name
                ),
            ));
            chronicle.record(Event::new(
                tick,
                format!("*** The Fall of {} ***", target_name),
            ));
            let _ = (t_col, t_row);
            // Record enmity on raider (target is gone).
            if let Some(r) = settlements.list.iter_mut().find(|s| s.id == raider_id) {
                r.note_raid(target_id);
            }
        } else if success {
            // Mark the top warrior of the raider as having led this raid.
            mark_raid_leader(agents, members_by_settlement, raider_id);
            // Loot: take a chunk of defender stockpile.
            let taken = {
                let t = settlements
                    .list
                    .iter_mut()
                    .find(|s| s.id == target_id)
                    .unwrap();
                let take = (t.stockpile * 0.5).min(20.0);
                t.stockpile -= take;
                take
            };
            if let Some(r) = settlements.list.iter_mut().find(|s| s.id == raider_id) {
                r.stockpile += taken;
            }
            chronicle.record(Event::new(
                tick,
                format!(
                    "{} sacks the granary of {}, carrying off their stores.",
                    raider_name, target_name
                ),
            ));
            let atk_losses = rng.gen_range(0..=1);
            let def_losses = rng.gen_range(1..=2).min(defenders.max(1));
            resolve_raid_outcome(
                settlements,
                agents,
                members_by_settlement,
                raider_id,
                target_id,
                &raider_name,
                &target_name,
                atk_losses,
                def_losses,
                chronicle,
                tick,
            );
        } else {
            chronicle.record(Event::new(
                tick,
                format!(
                    "The defenders of {} repel the raiders with heavy losses.",
                    target_name
                ),
            ));
            let atk_losses = rng.gen_range(2..=3).min(attackers);
            let def_losses = rng.gen_range(0..=1);
            resolve_raid_outcome(
                settlements,
                agents,
                members_by_settlement,
                raider_id,
                target_id,
                &raider_name,
                &target_name,
                atk_losses,
                def_losses,
                chronicle,
                tick,
            );
        }

        // Combat experience: every surviving warrior who took the field
        // sharpens their skill, including allied defenders who answered the
        // call. Defenders of the target also include non-warriors — the
        // raid was at their doorstep, so civilians learn to fight too, and
        // this is the hook that bootstraps new warriors.
        grant_combat_experience(agents, members_by_settlement, raider_id, true, chronicle, tick);
        grant_combat_experience(agents, members_by_settlement, target_id, false, chronicle, tick);
        for &aid in &target_allies {
            grant_combat_experience(agents, members_by_settlement, aid, true, chronicle, tick);
        }

        // Trait emergence after any raid outcome.
        let lines: Vec<String> = [raider_id, target_id]
            .iter()
            .filter_map(|&sid| {
                settlements
                    .list
                    .iter_mut()
                    .find(|s| s.id == sid && s.alive)
                    .and_then(|s| s.maybe_emerge_trait())
            })
            .collect();
        for line in lines {
            chronicle.record(Event::new(tick, line));
        }
    }
}

/// Scan fertility around each settlement and record depletion/recovery events.
fn update_land_health(
    settlements: &mut Settlements,
    world: &World,
    chronicle: &mut Chronicle,
    tick: u64,
) {
    const RADIUS: i32 = 3;
    const DEPLETE_THRESHOLD: f32 = 0.3;
    const RECOVER_THRESHOLD: f32 = 0.7;
    let year = tick / TICKS_PER_YEAR;

    for s in settlements.list.iter_mut() {
        if !s.alive {
            continue;
        }
        // Only consider biomes that can meaningfully deplete — deserts and
        // tundra are already near-barren by nature.
        let mut min_fert: f32 = f32::INFINITY;
        for dr in -RADIUS..=RADIUS {
            for dc in -RADIUS..=RADIUS {
                let c = s.col + dc;
                let r = s.row + dr;
                if world.hex_distance((s.col, s.row), (c, r)) > RADIUS {
                    continue;
                }
                if let Some(t) = world.tile(c, r) {
                    if t.biome.natural_fertility() > 0.5 {
                        min_fert = min_fert.min(t.fertility);
                    }
                }
            }
        }
        if !min_fert.is_finite() {
            continue;
        }

        // At most one land-health line per settlement per year.
        if s.last_land_event_year == Some(year) {
            continue;
        }

        if !s.land_depleted && min_fert < DEPLETE_THRESHOLD {
            s.land_depleted = true;
            s.last_land_event_year = Some(year);
            chronicle.record(Event::new(
                tick,
                format!("The forests near {} grow thin from heavy use.", s.name),
            ));
        } else if s.land_depleted && min_fert > RECOVER_THRESHOLD {
            s.land_depleted = false;
            s.last_land_event_year = Some(year);
            chronicle.record(Event::new(
                tick,
                format!("The land around {} begins to heal.", s.name),
            ));
        }
    }
}

/// Is there a mountain tile within [`CUSTOM_MOUNTAIN_RADIUS`] hexes of (col, row)?
fn near_mountain(world: &World, col: i32, row: i32) -> bool {
    for dc in -CUSTOM_MOUNTAIN_RADIUS..=CUSTOM_MOUNTAIN_RADIUS {
        for dr in -CUSTOM_MOUNTAIN_RADIUS..=CUSTOM_MOUNTAIN_RADIUS {
            let c = col + dc;
            let r = row + dr;
            if world.hex_distance((col, row), (c, r)) > CUSTOM_MOUNTAIN_RADIUS {
                continue;
            }
            if let Some(t) = world.tile(c, r) {
                if t.biome == Biome::Mountains {
                    return true;
                }
            }
        }
    }
    false
}

/// Pick a custom name from a per-kind pool. Deterministic in the provided rng
/// so runs with the same seed produce the same traditions.
fn pick_custom_name(kind: CustomKind, rng: &mut ChaCha8Rng) -> String {
    let pool: &[&str] = match kind {
        CustomKind::HarvestFeast => &[
            "the Feast of the Full Silo",
            "the Harvest Gathering",
            "the Rite of the Overflowing Granary",
            "the Night of the Long Tables",
        ],
        CustomKind::WarriorRite => &[
            "the Blood Oath",
            "the Rite of the Iron Year",
            "the Warrior's Vigil",
            "the Hunt of Names",
        ],
        CustomKind::MemorialVigil => &[
            "the Silent Vigil",
            "the Night of Torches",
            "the Remembrance of the Burning",
            "the Day the Walls Wept",
        ],
        CustomKind::MerchantFair => &[
            "the Full-Moon Market",
            "the Caravan Circle",
            "the Traders' Gathering",
            "the Market at the Stone",
        ],
        CustomKind::RiverBlessing => &[
            "the Blessing of the Waters",
            "the Fishers' Song",
            "the Offering to the River",
            "the Rite of the Tides",
        ],
        CustomKind::MountainPilgrimage => &[
            "the Pilgrimage to the High Stones",
            "the Climb of the Elders",
            "the Journey Above the Clouds",
            "the Vigil upon the Peaks",
        ],
    };
    pool[rng.gen_range(0..pool.len())].to_string()
}

/// Chronicle line announcing that a custom has taken root in a settlement.
fn custom_emergence_line(settlement_name: &str, kind: CustomKind, custom_name: &str) -> String {
    let (verb_phrase, coda) = match kind {
        CustomKind::HarvestFeast => ("begin to keep", "a tradition born of plenty"),
        CustomKind::WarriorRite => ("take up", "a tradition forged in the bloody years"),
        CustomKind::MemorialVigil => ("begin to observe", "a tradition borne out of loss"),
        CustomKind::MerchantFair => (
            "establish",
            "a tradition grown from the coming and going of merchants",
        ),
        CustomKind::RiverBlessing => (
            "begin to hold",
            "a tradition shaped by the waters that feed them",
        ),
        CustomKind::MountainPilgrimage => (
            "take up",
            "a tradition whispered by the stones that ring their home",
        ),
    };
    format!(
        "The people of {} {} {} — {}.",
        settlement_name, verb_phrase, custom_name, coda
    )
}

/// Master pool of name prefixes. Each dialect draws a random subset from this
/// list, and when no dialect is available the full pool acts as the global
/// fallback.
const MASTER_PREFIXES: &[&str] = &[
    "Thorn", "Dusk", "Vel", "Ash", "El", "Ver", "Bryn", "Mor", "Kel", "Dun", "Hal", "Sten", "Wyn",
    "Gale", "Fro", "Cal", "Rav", "Iron", "Oak", "Stone", "Mar", "Fen", "Cor", "Drav", "Lyn", "Myr",
    "Nor", "Pen", "Ryn", "Shad", "Sil", "Tal", "Tor", "Ur", "Vane", "Wyr", "Yth", "Zar", "Brim",
    "Aln",
];
/// Master pool of name suffixes — paired with prefixes to compose settlement names.
const MASTER_SUFFIXES: &[&str] = &[
    "hold", "moor", "fall", "mara", "ford", "reach", "mere", "wick", "wold", "stead", "gate",
    "haven", "crag", "vale", "burn", "stow", "keep", "march", "dale", "ridge", "thorpe", "rock",
    "cove", "hollow", "shire", "wood", "glen", "spire", "bay", "barrow",
];

/// How many language centers to seed per world — a few regions with distinct
/// tongues yields clear linguistic boundaries without shattering the map into
/// unreadable pockets.
const DIALECT_CENTER_MIN: usize = 3;
const DIALECT_CENTER_MAX: usize = 6;
/// Each dialect draws this many prefixes / suffixes from the master pool.
/// A narrow-enough slice keeps names within a region feeling cohesive.
const DIALECT_PREFIX_MIN: usize = 8;
const DIALECT_PREFIX_MAX: usize = 12;
const DIALECT_SUFFIX_MIN: usize = 5;
const DIALECT_SUFFIX_MAX: usize = 8;

/// A per-region naming vocabulary. Two settlements sharing the same dialect
/// will sound like they came from the same people; two settlements with
/// different dialects won't.
#[derive(Debug, Clone)]
pub struct Dialect {
    pub prefixes: Vec<&'static str>,
    pub suffixes: Vec<&'static str>,
}

impl Dialect {
    fn from_master(rng: &mut ChaCha8Rng) -> Self {
        let pref_count = rng.gen_range(DIALECT_PREFIX_MIN..=DIALECT_PREFIX_MAX);
        let suf_count = rng.gen_range(DIALECT_SUFFIX_MIN..=DIALECT_SUFFIX_MAX);
        Self {
            prefixes: sample_pool(MASTER_PREFIXES, pref_count, rng),
            suffixes: sample_pool(MASTER_SUFFIXES, suf_count, rng),
        }
    }
}

/// A point on the map anchoring a [`Dialect`]. Settlements founded closest to
/// this center inherit its naming pool.
#[derive(Debug, Clone)]
pub struct LanguageCenter {
    pub col: i32,
    pub row: i32,
    pub dialect: Dialect,
}

/// The full set of language centers for a world. Centers are scattered at
/// world-gen time; the nearest one to a new settlement decides its dialect.
#[derive(Debug, Clone)]
pub struct Dialects {
    pub centers: Vec<LanguageCenter>,
}

impl Dialects {
    pub fn empty() -> Self {
        Self { centers: Vec::new() }
    }

    /// Scatter 3–6 language centers across the world's land tiles with a
    /// rough minimum separation so each center commands a meaningful region.
    /// Runs on a seeded RNG derived from the world's seed so dialects are
    /// deterministic without perturbing the main simulation RNG stream.
    pub fn generate(world: &World, seed: u64) -> Self {
        let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0xD1A1_EC75_EED_u64);
        let min_sep = ((world.width.min(world.height) as i32) / 3).max(5);
        let target = rng.gen_range(DIALECT_CENTER_MIN..=DIALECT_CENTER_MAX);
        let mut centers: Vec<LanguageCenter> = Vec::new();
        for _ in 0..500 {
            if centers.len() >= target {
                break;
            }
            let col = rng.gen_range(0..world.width as i32);
            let row = rng.gen_range(0..world.height as i32);
            if !world.is_land(col, row) {
                continue;
            }
            if centers
                .iter()
                .any(|c| world.hex_distance((c.col, c.row), (col, row)) < min_sep)
            {
                continue;
            }
            let dialect = Dialect::from_master(&mut rng);
            centers.push(LanguageCenter { col, row, dialect });
        }
        Self { centers }
    }

    /// Index of the language center nearest to (col, row), or `None` if the
    /// world has no centers (e.g. built via [`Self::empty`]).
    pub fn nearest(&self, world: &World, col: i32, row: i32) -> Option<usize> {
        self.centers
            .iter()
            .enumerate()
            .map(|(i, c)| (i, world.hex_distance((c.col, c.row), (col, row))))
            .min_by_key(|(_, d)| *d)
            .map(|(i, _)| i)
    }
}

/// Partial Fisher–Yates: pick `n` distinct entries from `pool` without
/// mutating it, consuming rng deterministically.
fn sample_pool(pool: &[&'static str], n: usize, rng: &mut ChaCha8Rng) -> Vec<&'static str> {
    let n = n.min(pool.len());
    let mut indices: Vec<usize> = (0..pool.len()).collect();
    for i in 0..n {
        let j = rng.gen_range(i..pool.len());
        indices.swap(i, j);
    }
    indices.into_iter().take(n).map(|i| pool[i]).collect()
}

/// Produce a settlement name. If `dialect` is provided its pools are used;
/// otherwise the full master pool acts as a fallback so code paths that
/// haven't wired up dialects still get sensible names.
fn generate_name(rng: &mut ChaCha8Rng, dialect: Option<&Dialect>) -> String {
    let (prefixes, suffixes): (&[&str], &[&str]) = match dialect {
        Some(d) if !d.prefixes.is_empty() && !d.suffixes.is_empty() => {
            (d.prefixes.as_slice(), d.suffixes.as_slice())
        }
        _ => (MASTER_PREFIXES, MASTER_SUFFIXES),
    };
    let p = prefixes[rng.gen_range(0..prefixes.len())];
    let s = suffixes[rng.gen_range(0..suffixes.len())];
    format!("{}{}", p, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;

    #[test]
    fn dialects_generate_populates_centers_on_a_normal_map() {
        let world = World::generate(80, 40, 2024);
        let dialects = Dialects::generate(&world, 2024);
        assert!(
            dialects.centers.len() >= DIALECT_CENTER_MIN,
            "expected at least {} centers, got {}",
            DIALECT_CENTER_MIN,
            dialects.centers.len()
        );
        for c in &dialects.centers {
            assert!(world.is_land(c.col, c.row), "center must sit on land");
            assert!(!c.dialect.prefixes.is_empty());
            assert!(!c.dialect.suffixes.is_empty());
        }
    }

    #[test]
    fn dialect_generation_is_deterministic_for_seed() {
        let world = World::generate(80, 40, 2024);
        let a = Dialects::generate(&world, 2024);
        let b = Dialects::generate(&world, 2024);
        assert_eq!(a.centers.len(), b.centers.len());
        for (ca, cb) in a.centers.iter().zip(b.centers.iter()) {
            assert_eq!((ca.col, ca.row), (cb.col, cb.row));
            assert_eq!(ca.dialect.prefixes, cb.dialect.prefixes);
            assert_eq!(ca.dialect.suffixes, cb.dialect.suffixes);
        }
    }

    #[test]
    fn distant_settlements_pick_different_dialects() {
        // Hand-built dialects pinned at opposite corners of the map so the
        // nearest-center lookup is unambiguous.
        let world = World::generate(80, 40, 42);
        let centers = vec![
            LanguageCenter {
                col: 5,
                row: 5,
                dialect: Dialect {
                    prefixes: vec!["Aa", "Bb"],
                    suffixes: vec!["x", "y"],
                },
            },
            LanguageCenter {
                col: 70,
                row: 35,
                dialect: Dialect {
                    prefixes: vec!["Cc", "Dd"],
                    suffixes: vec!["q", "r"],
                },
            },
        ];
        let dialects = Dialects { centers };
        let near_first = dialects.nearest(&world, 6, 6).unwrap();
        let near_second = dialects.nearest(&world, 69, 34).unwrap();
        assert_eq!(near_first, 0);
        assert_eq!(near_second, 1);
        assert_ne!(near_first, near_second);
    }

    #[test]
    fn generate_name_falls_back_to_master_pool_when_no_dialect() {
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let name = generate_name(&mut rng, None);
        assert!(
            MASTER_PREFIXES.iter().any(|p| name.starts_with(p)),
            "name {:?} should start with a master prefix",
            name
        );
        assert!(
            MASTER_SUFFIXES.iter().any(|s| name.ends_with(s)),
            "name {:?} should end with a master suffix",
            name
        );
    }

    #[test]
    fn generate_name_uses_dialect_pools_when_provided() {
        let dialect = Dialect {
            prefixes: vec!["Zy"],
            suffixes: vec!["zog"],
        };
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        for _ in 0..10 {
            let name = generate_name(&mut rng, Some(&dialect));
            assert_eq!(name, "Zyzog");
        }
    }
}
