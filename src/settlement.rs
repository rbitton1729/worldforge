use crate::agent::{Agent, MERCHANT_CARGO, MERCHANT_DISPATCH_THRESHOLD, MERCHANT_LOAD_MIN};
use crate::chronicle::{Chronicle, Event, TICKS_PER_YEAR};
use crate::world::World;
use rand::Rng;
use rand_chacha::ChaCha8Rng;

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
}

impl Settlements {
    pub fn new() -> Self {
        Self {
            list: Vec::new(),
            next_id: 0,
        }
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

    fn found(&mut self, col: i32, row: i32, tick: u64, rng: &mut ChaCha8Rng) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        let name = generate_name(rng);
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
    let mut best: Option<(usize, i32, i32, Vec<usize>)> = None;
    for &i in &unaffiliated {
        let a = &agents[i];
        let mut neighbors: Vec<usize> = Vec::new();
        for (j, other) in agents.iter().enumerate() {
            if !other.alive || other.is_traveling() || other.settlement.is_some() {
                continue;
            }
            if world.hex_distance((a.col, a.row), (other.col, other.row)) <= CLUSTER_RADIUS {
                neighbors.push(j);
            }
        }
        if neighbors.len() >= FOUND_THRESHOLD && !settlements.too_close(world, a.col, a.row) {
            let better = match &best {
                None => true,
                Some((n, _, _, _)) => neighbors.len() > *n,
            };
            if better {
                best = Some((neighbors.len(), a.col, a.row, neighbors));
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

        let id = settlements.found(ac, ar, tick, rng);
        for &j in &neighbors {
            agents[j].settlement = Some(id);
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

    // Raids between settlements.
    raid_phase(settlements, agents, world, rng, chronicle, tick);

    // Trade dispatch: settlements with surplus pick a skilled trader to send.
    try_dispatch_merchants(settlements, agents, rng);

    // Migration: if a settlement's people are starving, some depart to wander.
    migrate_from_starving(settlements, agents, world, rng, chronicle, tick);

    // Granary overflow chronicling — only during Autumn (season index 2).
    let season_idx = (tick % TICKS_PER_YEAR) / (TICKS_PER_YEAR / 4);
    for s in settlements.list.iter_mut() {
        if !s.alive {
            continue;
        }
        if s.stockpile > 40.0 && !s.overflow_declared {
            if season_idx == 2 {
                s.overflow_declared = true;
                chronicle.record(Event::new(
                    tick,
                    format!("The granary of {} overflows with autumn harvest.", s.name),
                ));
            }
        } else if s.stockpile < 20.0 && s.overflow_declared {
            s.overflow_declared = false;
        }
    }

    // Recount populations and retire any settlement that's lost all its people.
    for s in settlements.list.iter_mut() {
        if !s.alive {
            continue;
        }
        let pop = agents
            .iter()
            .filter(|a| a.alive && a.settlement == Some(s.id))
            .count() as u32;
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

    // Orphan agents whose settlement died.
    let dead_ids: Vec<u32> = settlements
        .list
        .iter()
        .filter(|s| !s.alive)
        .map(|s| s.id)
        .collect();
    if !dead_ids.is_empty() {
        for a in agents.iter_mut() {
            if let Some(sid) = a.settlement {
                if dead_ids.contains(&sid) {
                    a.settlement = None;
                }
            }
        }
    }
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
        let members: Vec<usize> = agents
            .iter()
            .enumerate()
            .filter(|(_, a)| a.alive && !a.is_traveling() && a.settlement == Some(sid))
            .map(|(i, _)| i)
            .collect();
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
        for (i, a) in agents.iter().enumerate() {
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
fn count_warriors(agents: &[Agent], sid: u32) -> u32 {
    agents
        .iter()
        .filter(|a| a.alive && a.is_warrior() && a.settlement == Some(sid))
        .count() as u32
}

/// Sum of fighting skill across all living warriors of settlement `sid`.
/// Used as the muster strength for raid resolution: a roomful of seasoned
/// warriors out-fights an equal count of journeymen.
fn warrior_strength(agents: &[Agent], sid: u32) -> f32 {
    agents
        .iter()
        .filter(|a| a.alive && a.is_warrior() && a.settlement == Some(sid))
        .map(|a| a.skills.fighting)
        .sum()
}

/// Kill up to `n` warriors belonging to settlement `sid`, returning how many fell.
fn slay_warriors(agents: &mut [Agent], sid: u32, n: u32) -> u32 {
    let mut killed = 0u32;
    for a in agents.iter_mut() {
        if killed >= n {
            break;
        }
        if a.alive && a.is_warrior() && a.settlement == Some(sid) {
            a.alive = false;
            killed += 1;
        }
    }
    killed
}

/// Reward all surviving warriors of `sid` with combat experience after a
/// raid, and emit a one-per-year chronicle line for any who just crossed
/// into "seasoned warrior" territory.
fn grant_combat_experience(
    agents: &mut [Agent],
    sid: u32,
    chronicle: &mut Chronicle,
    tick: u64,
) {
    use crate::agent::{FIGHTING_GROWTH, ROLE_RECOGNITION_THRESHOLD};
    let year = tick / crate::chronicle::TICKS_PER_YEAR;
    for a in agents.iter_mut() {
        if !a.alive || a.settlement != Some(sid) || !a.is_warrior() {
            continue;
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
    world: &World,
    rng: &mut ChaCha8Rng,
    chronicle: &mut Chronicle,
    tick: u64,
) {
    // Snapshot candidate raiders to avoid borrow issues during resolution.
    let candidates: Vec<(u32, i32, i32, f32)> = settlements
        .list
        .iter()
        .filter(|s| s.alive && s.stockpile < RAID_HUNGER_STOCK)
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
        let attackers = count_warriors(agents, raider_id);
        let attacker_strength = warrior_strength(agents, raider_id);
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

        let own_defenders = count_warriors(agents, target_id);
        let own_defender_strength = warrior_strength(agents, target_id);
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
            .map(|&aid| count_warriors(agents, aid))
            .sum();
        let ally_defender_strength: f32 = target_allies
            .iter()
            .filter(|&&aid| settlements.list.iter().any(|s| s.id == aid && s.alive))
            .map(|&aid| warrior_strength(agents, aid))
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
            slay_warriors(agents, target_id, defenders);
            // Surviving civilians of target lose affiliation. Their skills
            // travel with them — wherever they end up, they remember.
            for a in agents.iter_mut() {
                if a.alive && a.settlement == Some(target_id) {
                    a.settlement = None;
                    a.cargo = 0.0;
                    a.cargo_origin = None;
                    a.destination = None;
                }
            }
            // Attacker loses a couple of warriors even in victory.
            let atk_losses = rng.gen_range(0..=2).min(attackers.saturating_sub(1));
            slay_warriors(agents, raider_id, atk_losses);
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
            // Both sides suffer some warrior losses.
            let atk_losses = rng.gen_range(0..=1);
            let def_losses = rng.gen_range(1..=2).min(defenders.max(1));
            slay_warriors(agents, raider_id, atk_losses);
            slay_warriors(agents, target_id, def_losses);

            chronicle.record(Event::new(
                tick,
                format!(
                    "{} sacks the granary of {}, carrying off their stores.",
                    raider_name, target_name
                ),
            ));
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
        } else {
            // Repelled: attackers lose warriors, defenders lose a few too.
            let atk_losses = rng.gen_range(2..=3).min(attackers);
            let def_losses = rng.gen_range(0..=1);
            slay_warriors(agents, raider_id, atk_losses);
            slay_warriors(agents, target_id, def_losses);

            chronicle.record(Event::new(
                tick,
                format!(
                    "The defenders of {} repel the raiders with heavy losses.",
                    target_name
                ),
            ));
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

        // Combat experience: every surviving warrior who took the field
        // sharpens their skill, including allied defenders who answered the call.
        grant_combat_experience(agents, raider_id, chronicle, tick);
        grant_combat_experience(agents, target_id, chronicle, tick);
        for &aid in &target_allies {
            grant_combat_experience(agents, aid, chronicle, tick);
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

fn generate_name(rng: &mut ChaCha8Rng) -> String {
    const PREFIX: &[&str] = &[
        "Thorn", "Dusk", "Vel", "Ash", "El", "Ver", "Bryn", "Mor", "Kel", "Dun", "Hal", "Sten",
        "Wyn", "Gale", "Fro", "Cal", "Rav", "Iron", "Oak", "Stone", "Mar", "Fen",
    ];
    const SUFFIX: &[&str] = &[
        "hold", "moor", "fall", "mara", "ford", "reach", "mere", "wick", "wold", "stead", "gate",
        "haven", "crag", "vale", "burn", "stow",
    ];
    let p = PREFIX[rng.gen_range(0..PREFIX.len())];
    let s = SUFFIX[rng.gen_range(0..SUFFIX.len())];
    format!("{}{}", p, s)
}
