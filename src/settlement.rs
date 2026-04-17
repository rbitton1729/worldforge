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
            let id = settlements.found(a.col, a.row, tick, rng);
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
            chronicle.record(Event::new(
                tick,
                format!(
                    "A band of {} settlers gathers upon the {}. They name the place {}.",
                    neighbors.len(),
                    biome,
                    s.name
                ),
            ));
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
