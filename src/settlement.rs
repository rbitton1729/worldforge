use crate::agent::{Agent, WARRIOR_CHANCE};
use crate::chronicle::{Chronicle, Event};
use crate::world::World;
use rand::Rng;
use rand_chacha::ChaCha8Rng;

/// Trips on a declared route beyond which the two settlements pledge alliance.
const ALLIANCE_TRIPS: u32 = 8;
/// Maximum hex distance between settlements for a raid to be launched.
const RAID_MAX_DISTANCE: i32 = 12;
/// Stockpile below which a settlement becomes hungry enough to consider raiding.
const RAID_HUNGER_STOCK: f32 = 12.0;
/// Target must hold at least this much to be worth raiding.
const RAID_TARGET_STOCK: f32 = 25.0;
/// A raider settlement must muster this many warriors to attempt a raid.
const RAID_MIN_WARRIORS: u32 = 1;
/// Per-tick chance of rolling for a raid when conditions are met.
const RAID_CHANCE: f64 = 0.030;
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
}

/// Signals emitted when a trade trip is recorded.
#[derive(Debug, Clone, Copy, Default)]
pub struct TripSignal {
    pub road_formed: bool,
    pub alliance_formed: bool,
}

impl Settlement {
    pub fn note_trip(&mut self, other: u32) -> TripSignal {
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
        .filter(|(_, a)| a.alive && !a.merchant && a.settlement.is_none())
        .map(|(i, _)| i)
        .collect();

    for &i in &unaffiliated {
        let a = &agents[i];
        if !a.alive || a.settlement.is_some() {
            continue;
        }
        // Count nearby unaffiliated agents.
        let mut neighbors: Vec<usize> = Vec::new();
        for (j, other) in agents.iter().enumerate() {
            if !other.alive || other.merchant || other.settlement.is_some() {
                continue;
            }
            if world.hex_distance((a.col, a.row), (other.col, other.row)) <= CLUSTER_RADIUS {
                neighbors.push(j);
            }
        }
        if neighbors.len() >= FOUND_THRESHOLD && !settlements.too_close(world, a.col, a.row) {
            let (ac, ar) = (a.col, a.row);
            // Find nearest existing alive settlement before we add a new one.
            let nearest = settlements
                .list
                .iter()
                .filter(|s| s.alive)
                .map(|s| (s.name.clone(), world.hex_distance((s.col, s.row), (ac, ar))))
                .min_by_key(|(_, d)| *d);

            let id = settlements.found(ac, ar, tick, rng);
            for &j in &neighbors {
                agents[j].settlement = Some(id);
                // ~10% of founding members become merchants, ~15% warriors.
                if rng.gen_bool(0.10) {
                    agents[j].merchant = true;
                } else if rng.gen_bool(0.15) {
                    agents[j].warrior = true;
                }
            }
            let s = settlements
                .list
                .iter()
                .find(|s| s.id == id)
                .expect("just pushed");
            let biome = world
                .tile(s.col, s.row)
                .map(|t| t.biome.name())
                .unwrap_or("unknown land");
            let locator = match nearest {
                Some((other_name, d)) if d <= 15 => {
                    let days = describe_distance(d);
                    format!(" {} from {}", days, other_name)
                }
                _ => format!(" upon the {}", biome),
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
    }

    // Raids between settlements.
    raid_phase(settlements, agents, world, rng, chronicle, tick);

    // Migration: if a settlement's people are starving, some depart to wander.
    migrate_from_starving(settlements, agents, world, rng, chronicle, tick);

    // Granary overflow chronicling.
    for s in settlements.list.iter_mut() {
        if !s.alive {
            continue;
        }
        if s.stockpile > 40.0 && !s.overflow_declared {
            s.overflow_declared = true;
            chronicle.record(Event::new(
                tick,
                format!("The granary of {} overflows with autumn harvest.", s.name),
            ));
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
            .filter(|(_, a)| a.alive && !a.merchant && a.settlement == Some(sid))
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

/// Count living warriors affiliated with settlement `sid`.
fn count_warriors(agents: &[Agent], sid: u32) -> u32 {
    agents
        .iter()
        .filter(|a| a.alive && a.warrior && a.settlement == Some(sid))
        .count() as u32
}

/// Kill up to `n` warriors belonging to settlement `sid`, returning how many fell.
fn slay_warriors(agents: &mut [Agent], sid: u32, n: u32) -> u32 {
    let mut killed = 0u32;
    for a in agents.iter_mut() {
        if killed >= n {
            break;
        }
        if a.alive && a.warrior && a.settlement == Some(sid) {
            a.alive = false;
            killed += 1;
        }
    }
    killed
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
        // Re-check raider (prior iterations may have destroyed it).
        if !settlements.list.iter().any(|s| s.id == raider_id && s.alive) {
            continue;
        }
        let attackers = count_warriors(agents, raider_id);
        if attackers < RAID_MIN_WARRIORS {
            continue;
        }
        if !rng.gen_bool(RAID_CHANCE) {
            continue;
        }

        // Pick the richest non-allied neighbor in range.
        let raider_allied: Vec<u32> = settlements
            .list
            .iter()
            .find(|s| s.id == raider_id)
            .map(|s| s.routes.iter().filter(|r| r.allied).map(|r| r.other_id).collect())
            .unwrap_or_default();

        let target_opt = settlements
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
            .map(|s| s.id);

        let Some(target_id) = target_opt else { continue };

        let defenders = count_warriors(agents, target_id);

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

        // Resolve: attacker roll vs defender roll + home advantage.
        let atk_roll = attackers as f32 + rng.gen_range(0.0..3.0);
        let def_roll = defenders as f32 + 1.0 + rng.gen_range(0.0..3.0);

        let sack = attackers >= 3 && atk_roll >= def_roll * 2.0;
        let success = atk_roll > def_roll;

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
            // Surviving civilians of target lose affiliation.
            for a in agents.iter_mut() {
                if a.alive && a.settlement == Some(target_id) {
                    a.settlement = None;
                    a.merchant = false;
                    a.warrior = false;
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
