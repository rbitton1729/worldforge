use crate::chronicle::{Chronicle, Event};
use crate::world::World;
use rand::Rng;
use rand_chacha::ChaCha8Rng;

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
    pub col: i32,
    pub row: i32,
    pub hunger: f32,
    pub health: f32,
    pub age: u32,
    pub max_age: u32,
    pub alive: bool,
    pub settlement: Option<u32>,
}

impl Agent {
    pub fn new(id: u32, col: i32, row: i32, max_age: u32) -> Self {
        Self {
            id,
            col,
            row,
            hunger: 20.0,
            health: 100.0,
            age: 0,
            max_age,
            alive: true,
            settlement: None,
        }
    }
}

fn roll_lifespan(rng: &mut ChaCha8Rng) -> u32 {
    LIFESPAN_BASE + rng.gen_range(0..=LIFESPAN_VARIANCE)
}

/// Run one tick for every agent: update hunger, forage or wander, resolve deaths.
pub fn step_agents(
    agents: &mut Vec<Agent>,
    world: &mut World,
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

        // Try to eat first if we're standing on food.
        let on_food = world
            .tile(agent.col, agent.row)
            .map_or(false, |t| t.food >= 0.5);

        if on_food && (starving || agent.hunger > 30.0) {
            if let Some(t) = world.tile_mut(agent.col, agent.row) {
                let bite = BITE_SIZE.min(t.food);
                t.food -= bite;
                agent.hunger = (agent.hunger - bite * FOOD_TO_HUNGER).max(0.0);
            }
        } else {
            // Move: seek food if starving, else wander.
            let target = if starving {
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
                    "Soul #{} dies of old age at {} years, on the {} near ({}, {}).",
                    agent.id,
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
                    "Soul #{} perishes of hunger on the {} near ({}, {}).",
                    agent.id,
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
                let mut child = Agent::new(next_id, agent.col, agent.row, roll_lifespan(rng));
                child.hunger = 35.0;
                child.settlement = agent.settlement;
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
                let mut agent = Agent::new(placed, col, row, roll_lifespan(rng));
                agent.age = rng.gen_range(0..LIFESPAN_BASE / 2);
                out.push(agent);
                placed += 1;
            }
        }
    }
    out
}
