use crate::agent::Agent;
use crate::chronicle::{Chronicle, Event};
use crate::world::World;
use rand::Rng;
use rand_chacha::ChaCha8Rng;

/// Minimum loyal / nearby agents required to found a settlement.
const FOUND_THRESHOLD: usize = 5;
/// Radius (in hexes) within which agents count as "together".
const CLUSTER_RADIUS: i32 = 2;
/// Don't found a new settlement within this many hexes of an existing one.
const MIN_SEPARATION: i32 = 6;

#[derive(Debug, Clone)]
pub struct Settlement {
    pub id: u32,
    pub name: String,
    pub col: i32,
    pub row: i32,
    pub founded_tick: u64,
    pub population: u32,
    pub alive: bool,
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
        .filter(|(_, a)| a.alive && a.settlement.is_none())
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
            if !other.alive || other.settlement.is_some() {
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

    // Migration: if a settlement's people are starving, some depart to wander.
    migrate_from_starving(settlements, agents, world, rng, chronicle, tick);

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
            .filter(|(_, a)| a.alive && a.settlement == Some(sid))
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
