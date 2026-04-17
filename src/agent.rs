use crate::chronicle::{Chronicle, Event};
use crate::settlement::Settlements;
use crate::world::{World, FERTILITY_PER_BITE};
use rand::Rng;
use rand_chacha::ChaCha8Rng;

/// Probability a newly-born/seeded agent becomes a merchant.
const MERCHANT_CHANCE: f64 = 0.10;
/// Probability a newly-born/settlement-joining agent becomes a warrior.
pub const WARRIOR_CHANCE: f64 = 0.10;
/// Warriors try to stay within this hex radius of their settlement.
const WARRIOR_PATROL_RADIUS: i32 = 3;
/// Food units a merchant loads from a settlement stockpile in one trip.
const MERCHANT_CARGO: f32 = 4.0;
/// Minimum stockpile before a merchant will load cargo and depart.
const MERCHANT_LOAD_MIN: f32 = 6.0;

/// Hunger above this → starving, agents prioritize food.
pub const HUNGER_STARVE_THRESHOLD: f32 = 70.0;
/// Hunger at 100 means the agent starts taking health damage.
pub const HUNGER_MAX: f32 = 100.0;
/// Hunger increase per tick.
const HUNGER_PER_TICK: f32 = 1.4;
/// Health damage per tick while fully starving.
const STARVE_DAMAGE: f32 = 3.0;

/// Agents well-fed below this hunger value can reproduce.
const REPRO_HUNGER_MAX: f32 = 28.0;
/// Minimum age before reproduction.
const REPRO_MIN_AGE: u32 = 60;
/// Per-tick probability of reproducing when conditions are met.
const REPRO_CHANCE: f64 = 0.010;
/// Radius within which another agent counts as a neighbor for reproduction.
const REPRO_RADIUS: i32 = 1;

/// Baseline lifespan in ticks (100 ticks = 1 year).
const LIFESPAN_BASE: u32 = 600;
/// Random extra lifespan applied per agent.
const LIFESPAN_VARIANCE: u32 = 300;
/// Food eaten per bite (per tick on a food-bearing tile).
const BITE_SIZE: f32 = 2.0;
/// How much hunger one unit of food restores.
const FOOD_TO_HUNGER: f32 = 15.0;

#[derive(Debug, Clone)]
pub struct Agent {
    pub id: u32,
    pub name: String,
    pub col: i32,
    pub row: i32,
    pub hunger: f32,
    pub health: f32,
    pub age: u32,
    pub max_age: u32,
    pub alive: bool,
    pub settlement: Option<u32>,
    pub merchant: bool,
    pub warrior: bool,
    pub cargo: f32,
    pub cargo_origin: Option<u32>,
    pub destination: Option<u32>,
}

impl Agent {
    pub fn new(id: u32, name: String, col: i32, row: i32, max_age: u32) -> Self {
        Self {
            id,
            name,
            col,
            row,
            hunger: 20.0,
            health: 100.0,
            age: 0,
            max_age,
            alive: true,
            settlement: None,
            merchant: false,
            warrior: false,
            cargo: 0.0,
            cargo_origin: None,
            destination: None,
        }
    }
}

const FIRST_NAMES: &[&str] = &[
    "Elara", "Bran", "Cael", "Dara", "Eryn", "Fenn", "Gwyn", "Halla", "Ivor", "Jora",
    "Kiran", "Lyra", "Maren", "Nyx", "Oren", "Perrin", "Quill", "Rhea", "Soren", "Tamsin",
    "Ulric", "Vesna", "Wyl", "Xan", "Yarrow", "Zephyr", "Alden", "Briar", "Corin", "Doran",
    "Eira", "Faye", "Gale", "Hollis", "Isolde", "Jareth", "Kestrel", "Linnea", "Merrick", "Nerys",
    "Osric", "Piran", "Rowan", "Saela", "Torren", "Una", "Vale", "Wren", "Yves", "Zinna",
    "Astra", "Bryn", "Caden", "Delia", "Emric", "Fable", "Garrick", "Hale", "Indra", "Joren",
    "Kael", "Lune", "Mira", "Nolan", "Orla", "Phelan", "Rune", "Sable", "Thane", "Ursa",
    "Varen", "Willa", "Yorick", "Zara",
];

pub fn pick_name(rng: &mut ChaCha8Rng) -> String {
    FIRST_NAMES[rng.gen_range(0..FIRST_NAMES.len())].to_string()
}

fn roll_lifespan(rng: &mut ChaCha8Rng) -> u32 {
    LIFESPAN_BASE + rng.gen_range(0..=LIFESPAN_VARIANCE)
}

/// Run one tick for every agent: update hunger, forage or wander, resolve deaths.
pub fn step_agents(
    agents: &mut Vec<Agent>,
    world: &mut World,
    settlements: &mut Settlements,
    rng: &mut ChaCha8Rng,
    chronicle: &mut Chronicle,
    tick: u64,
) {
    // Snapshot living positions for reproduction neighbor checks.
    let living_positions: Vec<(i32, i32)> = agents
        .iter()
        .filter(|a| a.alive)
        .map(|a| (a.col, a.row))
        .collect();

    let mut next_id = agents.iter().map(|a| a.id).max().unwrap_or(0) + 1;
    let mut newborns: Vec<Agent> = Vec::new();

    for agent in agents.iter_mut() {
        if !agent.alive {
            continue;
        }

        agent.age += 1;
        agent.hunger = (agent.hunger + HUNGER_PER_TICK).min(HUNGER_MAX);

        let starving = agent.hunger >= HUNGER_STARVE_THRESHOLD;

        if agent.merchant {
            step_merchant(agent, world, settlements, rng, chronicle, tick);
        } else {
            // Try to eat first if we're standing on food.
            let on_food = world
                .tile(agent.col, agent.row)
                .map_or(false, |t| t.food >= 0.5);

            if on_food && (starving || agent.hunger > 30.0) {
                if let Some(t) = world.tile_mut(agent.col, agent.row) {
                    let bite = BITE_SIZE.min(t.food);
                    t.food -= bite;
                    t.fertility =
                        (t.fertility - FERTILITY_PER_BITE * (bite / BITE_SIZE)).max(0.0);
                    agent.hunger = (agent.hunger - bite * FOOD_TO_HUNGER).max(0.0);
                }
            } else {
                // Warriors patrol near their settlement instead of ranging for food.
                let home = agent.settlement.and_then(|sid| {
                    settlements
                        .list
                        .iter()
                        .find(|s| s.id == sid && s.alive)
                        .map(|s| (s.col, s.row))
                });
                let far_from_home = match (agent.warrior, home) {
                    (true, Some((hc, hr))) => {
                        world.hex_distance((agent.col, agent.row), (hc, hr))
                            > WARRIOR_PATROL_RADIUS
                    }
                    _ => false,
                };

                // Move: patrol home, seek food if starving, else wander.
                let target = if far_from_home {
                    home
                } else if starving {
                    find_nearby_food(world, agent.col, agent.row, 3)
                } else {
                    None
                };

                let (nc, nr) = match target {
                    Some((tc, tr)) => step_toward(world, agent.col, agent.row, tc, tr),
                    None => wander(world, agent.col, agent.row, rng),
                };

                if world.is_land(nc, nr) {
                    agent.col = nc;
                    agent.row = nr;
                }
            }

            // Settled foragers gather surplus for the stockpile.
            if agent.hunger < 30.0 {
                if let Some(sid) = agent.settlement {
                    let tile_food = world.tile(agent.col, agent.row).map(|t| t.food).unwrap_or(0.0);
                    if tile_food >= 1.0 {
                        let gather = 0.5_f32.min(tile_food - 0.5);
                        if gather > 0.0 {
                            if let Some(t) = world.tile_mut(agent.col, agent.row) {
                                t.food -= gather;
                                t.fertility = (t.fertility
                                    - FERTILITY_PER_BITE * (gather / BITE_SIZE))
                                    .max(0.0);
                            }
                            if let Some(s) = settlements.list.iter_mut().find(|s| s.id == sid) {
                                s.stockpile += gather;
                            }
                        }
                    }
                }
            }

            // Starving settled agents can eat from the stockpile if close to home.
            if starving {
                if let Some(sid) = agent.settlement {
                    if let Some(s) = settlements.list.iter_mut().find(|s| s.id == sid) {
                        if world.hex_distance((s.col, s.row), (agent.col, agent.row)) <= 1
                            && s.stockpile >= 0.5
                        {
                            let bite = BITE_SIZE.min(s.stockpile);
                            s.stockpile -= bite;
                            agent.hunger = (agent.hunger - bite * FOOD_TO_HUNGER).max(0.0);
                        }
                    }
                }
            }
        }

        // Starvation damage.
        if agent.hunger >= HUNGER_MAX {
            agent.health -= STARVE_DAMAGE;
        } else if agent.health < 100.0 && agent.hunger < 40.0 {
            agent.health = (agent.health + 0.5).min(100.0);
        }

        // Death from old age.
        if agent.alive && agent.age >= agent.max_age {
            agent.alive = false;
            chronicle.record(Event::new(
                tick,
                format!(
                    "{} dies of old age at {} years, on the {} near ({}, {}).",
                    agent.name,
                    agent.age as u64 / crate::chronicle::TICKS_PER_YEAR,
                    world
                        .tile(agent.col, agent.row)
                        .map(|t| t.biome.name())
                        .unwrap_or("void"),
                    agent.col,
                    agent.row
                ),
            ));
            continue;
        }

        if agent.health <= 0.0 {
            agent.alive = false;
            chronicle.record(Event::new(
                tick,
                format!(
                    "{} perishes of hunger on the {} near ({}, {}).",
                    agent.name,
                    world
                        .tile(agent.col, agent.row)
                        .map(|t| t.biome.name())
                        .unwrap_or("void"),
                    agent.col,
                    agent.row
                ),
            ));
            continue;
        }

        // Reproduction: well-fed, mature agent near another agent.
        if agent.hunger <= REPRO_HUNGER_MAX
            && agent.age >= REPRO_MIN_AGE
            && rng.gen_bool(REPRO_CHANCE)
        {
            let has_partner = living_positions.iter().any(|&(c, r)| {
                (c, r) != (agent.col, agent.row)
                    && world.hex_distance((agent.col, agent.row), (c, r)) <= REPRO_RADIUS
            });
            if has_partner {
                let mut child = Agent::new(
                    next_id,
                    pick_name(rng),
                    agent.col,
                    agent.row,
                    roll_lifespan(rng),
                );
                child.hunger = 35.0;
                child.settlement = agent.settlement;
                if agent.settlement.is_some() && rng.gen_bool(MERCHANT_CHANCE) {
                    child.merchant = true;
                } else if agent.settlement.is_some() && rng.gen_bool(WARRIOR_CHANCE) {
                    child.warrior = true;
                }
                next_id += 1;
                // Parent pays a hunger cost for bearing a child.
                agent.hunger = (agent.hunger + 25.0).min(HUNGER_MAX);
                newborns.push(child);
            }
        }
    }

    if !newborns.is_empty() {
        agents.extend(newborns);
    }
}

/// Handle one tick for a merchant agent: load cargo at home, travel to
/// destination, deposit cargo, then declare the destination the new home.
fn step_merchant(
    agent: &mut Agent,
    world: &mut World,
    settlements: &mut Settlements,
    rng: &mut ChaCha8Rng,
    chronicle: &mut Chronicle,
    tick: u64,
) {
    // If the home settlement is gone, demote to a regular wanderer.
    let home_alive = agent
        .settlement
        .and_then(|id| settlements.list.iter().find(|s| s.id == id))
        .map_or(false, |s| s.alive);
    if !home_alive {
        agent.merchant = false;
        agent.cargo = 0.0;
        agent.destination = None;
        agent.cargo_origin = None;
        agent.settlement = None;
        return;
    }

    // Eat from cargo when hungry, so merchants don't starve on the road.
    if agent.hunger >= 50.0 && agent.cargo >= 0.5 {
        let bite = BITE_SIZE.min(agent.cargo);
        agent.cargo -= bite;
        agent.hunger = (agent.hunger - bite * FOOD_TO_HUNGER).max(0.0);
    } else if agent.hunger >= HUNGER_STARVE_THRESHOLD {
        // Desperate: try to eat from the ground.
        let tile_food = world.tile(agent.col, agent.row).map(|t| t.food).unwrap_or(0.0);
        if tile_food >= 0.5 {
            if let Some(t) = world.tile_mut(agent.col, agent.row) {
                let bite = BITE_SIZE.min(t.food);
                t.food -= bite;
                t.fertility =
                    (t.fertility - FERTILITY_PER_BITE * (bite / BITE_SIZE)).max(0.0);
                agent.hunger = (agent.hunger - bite * FOOD_TO_HUNGER).max(0.0);
            }
        }
    }

    // If carrying cargo toward a destination, travel; if arrived, deliver.
    if let Some(dest_id) = agent.destination {
        let dest = settlements
            .list
            .iter()
            .find(|s| s.id == dest_id && s.alive)
            .map(|s| (s.col, s.row, s.name.clone()));
        match dest {
            Some((dc, dr, dname)) => {
                if (agent.col, agent.row) == (dc, dr) {
                    let origin_id = agent.cargo_origin;
                    let origin_name = origin_id
                        .and_then(|oid| settlements.list.iter().find(|s| s.id == oid))
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| "distant lands".to_string());
                    let origin_stock = origin_id
                        .and_then(|oid| settlements.list.iter().find(|s| s.id == oid))
                        .map(|s| s.stockpile)
                        .unwrap_or(0.0);
                    let delivered = agent.cargo;
                    if let Some(dest_s) =
                        settlements.list.iter_mut().find(|s| s.id == dest_id)
                    {
                        let needy = dest_s.stockpile < origin_stock + 1.0;
                        if delivered >= 0.5 && needy {
                            dest_s.stockpile += delivered;
                            // Occasional chronicle of arrival keeps the log from flooding.
                            if rng.gen_bool(0.2) {
                                chronicle.record(Event::new(
                                    tick,
                                    format!(
                                        "A merchant arrives at {} bearing grain from distant {}.",
                                        dname, origin_name
                                    ),
                                ));
                            }
                        }
                    }
                    // Record the trip on both endpoints, and chronicle a new trade road once.
                    let mut road_formed = false;
                    let mut alliance_formed = false;
                    if let Some(oid) = origin_id {
                        if let Some(o) = settlements.list.iter_mut().find(|s| s.id == oid) {
                            let s = o.note_trip(dest_id);
                            road_formed |= s.road_formed;
                            alliance_formed |= s.alliance_formed;
                        }
                        if let Some(d) = settlements.list.iter_mut().find(|s| s.id == dest_id) {
                            let s = d.note_trip(oid);
                            road_formed |= s.road_formed;
                            alliance_formed |= s.alliance_formed;
                        }
                    }
                    if road_formed {
                        chronicle.record(Event::new(
                            tick,
                            format!(
                                "A trade road forms between {} and {}.",
                                origin_name, dname
                            ),
                        ));
                    }
                    if alliance_formed {
                        chronicle.record(Event::new(
                            tick,
                            format!(
                                "{} and {} pledge mutual defense.",
                                origin_name, dname
                            ),
                        ));
                    }
                    // Trait emergence: check both endpoints after the trade.
                    if let Some(oid) = origin_id {
                        let lines: Vec<String> = [oid, dest_id]
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
                    agent.cargo = 0.0;
                    agent.cargo_origin = None;
                    agent.destination = None;
                    agent.settlement = Some(dest_id);
                } else {
                    let (nc, nr) = step_toward(world, agent.col, agent.row, dc, dr);
                    if world.is_land(nc, nr) {
                        agent.col = nc;
                        agent.row = nr;
                    }
                }
            }
            None => {
                // Destination vanished; drop it and idle.
                agent.destination = None;
                agent.cargo = 0.0;
                agent.cargo_origin = None;
            }
        }
        return;
    }

    // No destination: at home, try to load and pick a new route.
    let home_id = agent.settlement.unwrap();
    let (hc, hr, stock) = settlements
        .list
        .iter()
        .find(|s| s.id == home_id)
        .map(|s| (s.col, s.row, s.stockpile))
        .unwrap();

    if (agent.col, agent.row) == (hc, hr) {
        if stock >= MERCHANT_LOAD_MIN {
            let others: Vec<u32> = settlements
                .list
                .iter()
                .filter(|s| s.alive && s.id != home_id)
                .map(|s| s.id)
                .collect();
            if !others.is_empty() {
                let dest_id = others[rng.gen_range(0..others.len())];
                let load = MERCHANT_CARGO.min(stock);
                if let Some(h) = settlements.list.iter_mut().find(|s| s.id == home_id) {
                    h.stockpile -= load;
                }
                agent.cargo = load;
                agent.cargo_origin = Some(home_id);
                agent.destination = Some(dest_id);
            }
        }
    } else {
        let (nc, nr) = step_toward(world, agent.col, agent.row, hc, hr);
        if world.is_land(nc, nr) {
            agent.col = nc;
            agent.row = nr;
        }
    }
}

/// Scan hex tiles within `radius` for the one with the most food; return its coords.
fn find_nearby_food(world: &World, col: i32, row: i32, radius: i32) -> Option<(i32, i32)> {
    let mut best: Option<((i32, i32), f32)> = None;
    for dr in -radius..=radius {
        for dc in -radius..=radius {
            let c = col + dc;
            let r = row + dr;
            if world.hex_distance((col, row), (c, r)) > radius {
                continue;
            }
            if let Some(tile) = world.tile(c, r) {
                if tile.food >= 1.0 {
                    let score = tile.food - world.hex_distance((col, row), (c, r)) as f32 * 0.5;
                    match best {
                        Some((_, s)) if s >= score => {}
                        _ => best = Some(((c, r), score)),
                    }
                }
            }
        }
    }
    best.map(|(pos, _)| pos)
}

/// Pick the neighbor that reduces hex distance to the target the most.
fn step_toward(world: &World, col: i32, row: i32, tc: i32, tr: i32) -> (i32, i32) {
    let cur = world.hex_distance((col, row), (tc, tr));
    let mut best = (col, row);
    let mut best_d = cur;
    for (nc, nr) in world.neighbors(col, row) {
        if !world.is_land(nc, nr) {
            continue;
        }
        let d = world.hex_distance((nc, nr), (tc, tr));
        if d < best_d {
            best_d = d;
            best = (nc, nr);
        }
    }
    best
}

fn wander(world: &World, col: i32, row: i32, rng: &mut ChaCha8Rng) -> (i32, i32) {
    // 40% stay put, 60% step to a random passable neighbor.
    if rng.gen_bool(0.4) {
        return (col, row);
    }
    let neighbors = world.neighbors(col, row);
    let passable: Vec<(i32, i32)> = neighbors
        .into_iter()
        .filter(|&(c, r)| world.is_land(c, r))
        .collect();
    if passable.is_empty() {
        (col, row)
    } else {
        passable[rng.gen_range(0..passable.len())]
    }
}

pub fn alive_count(agents: &[Agent]) -> usize {
    agents.iter().filter(|a| a.alive).count()
}

/// Seed `n` agents randomly on passable, food-bearing tiles.
pub fn seed_agents(world: &World, n: u32, rng: &mut ChaCha8Rng) -> Vec<Agent> {
    let mut out = Vec::with_capacity(n as usize);
    let mut placed = 0u32;
    let mut attempts = 0u32;
    let max_attempts = n.saturating_mul(50).max(5000);
    while placed < n && attempts < max_attempts {
        attempts += 1;
        let col = rng.gen_range(0..world.width as i32);
        let row = rng.gen_range(0..world.height as i32);
        if let Some(tile) = world.tile(col, row) {
            if tile.biome.is_passable() && tile.biome.food_cap() > 0.0 {
                // Stagger starting ages so the founding generation doesn't all die at once.
                let mut agent = Agent::new(placed, pick_name(rng), col, row, roll_lifespan(rng));
                agent.age = rng.gen_range(0..LIFESPAN_BASE / 2);
                out.push(agent);
                placed += 1;
            }
        }
    }
    out
}
