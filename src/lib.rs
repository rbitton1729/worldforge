pub mod agent;
pub mod chronicle;
pub mod region;
pub mod settlement;
pub mod world;
pub mod worldgen;

use agent::{alive_count, seed_agents, step_agents, Agent};
use chronicle::{Chronicle, Event};
use rand_chacha::ChaCha8Rng;
use rand::SeedableRng;
use settlement::{update_settlements, Settlements};
use std::time::{Duration, Instant};
use world::{Biome, World};

pub struct SimConfig {
    pub seed: u64,
    pub width: u32,
    pub height: u32,
    pub agents: u32,
    pub ticks: u64,
    /// If set, pace each tick to roughly this many ticks per second (real time).
    /// Tests leave this `None` to run flat-out.
    pub tick_rate: Option<f64>,
}

impl SimConfig {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            width: 80,
            height: 40,
            agents: 200,
            ticks: 200,
            tick_rate: None,
        }
    }
}

pub struct SimOutcome {
    pub world: World,
    pub agents: Vec<Agent>,
    pub settlements: Settlements,
    pub final_tick: u64,
}

/// Run a simulation to completion (either all agents dead or `cfg.ticks` reached).
/// All narration is written to `chronicle`. Callers construct and configure the
/// chronicle themselves — stdout, file, or sink — and apply any color overrides
/// before handing it in.
pub fn run_simulation(cfg: SimConfig, chronicle: &mut Chronicle) -> SimOutcome {
    let mut rng = ChaCha8Rng::seed_from_u64(cfg.seed);

    let _ = chronicle.proclaim(&format!(
        "worldforge — seed {} — {}×{} world — {} souls",
        cfg.seed, cfg.width, cfg.height, cfg.agents
    ));

    let mut world = World::generate(cfg.width, cfg.height, cfg.seed);
    let mut agents: Vec<Agent> = seed_agents(&world, cfg.agents, &mut rng);
    let actual = agents.len();
    if actual < cfg.agents as usize {
        let _ = chronicle.proclaim(&format!(
            "The world could only bear {} of the hoped-for {} souls — much of it is sea and stone.",
            actual, cfg.agents
        ));
    }

    let biome_summary = summarize_biomes(&world);
    let river_clause = match world.river_count() {
        0 => String::from("No rivers cross the land"),
        1 => String::from("A lone river winds toward the sea"),
        n => format!("{} rivers wind toward the sea", n),
    };
    let _ = chronicle.proclaim(&format!(
        "The world takes shape: {}. {}. {} souls draw their first breath.",
        biome_summary, river_clause, actual
    ));
    let region_clause = describe_major_regions(&world);
    if !region_clause.is_empty() {
        let _ = chronicle.proclaim(&region_clause);
    }

    let mut settlements = Settlements::new();

    chronicle.record(Event::new(
        0,
        "The world awakens. Scattered bands wander the land in search of food.".to_string(),
    ));
    let _ = chronicle.flush_tick(0);

    let tick_duration = cfg
        .tick_rate
        .filter(|r| *r > 0.0)
        .map(|r| Duration::from_secs_f64(1.0 / r));

    let mut last_population_reported = actual;
    let mut last_report_year: u64 = 0;

    let mut tick: u64 = 0;
    loop {
        if cfg.ticks > 0 && tick >= cfg.ticks {
            break;
        }
        tick += 1;
        let tick_start = Instant::now();

        if let Some(line) = world.tick_climate(tick) {
            chronicle.record(Event::new(tick, line.to_string()));
        }
        world.regen_food(tick);
        step_agents(
            &mut agents,
            &mut world,
            &mut settlements,
            &mut rng,
            chronicle,
            tick,
        );
        update_settlements(
            &mut settlements,
            &mut agents,
            &world,
            &mut rng,
            chronicle,
            tick,
        );

        chronicle.set_header_stats(alive_count(&agents), settlements.alive_count());

        // Settlement population reports twice a year (every 2 seasons = 50 ticks).
        if tick % (chronicle::TICKS_PER_YEAR / 2) == 0 {
            report_settlements(&settlements, chronicle, tick);
        }

        // Once per year, report on the state of the world if it's changed meaningfully.
        let year = tick / chronicle::TICKS_PER_YEAR + 1;
        if tick % chronicle::TICKS_PER_YEAR == 0 && year != last_report_year {
            last_report_year = year;
            let pop = alive_count(&agents);
            let delta = pop as isize - last_population_reported as isize;
            if delta.abs() >= (last_population_reported as isize / 10).max(5) {
                let verb = if delta > 0 { "swell" } else { "dwindle" };
                chronicle.record(Event::new(
                    tick,
                    format!(
                        "Across the land the living number {} — they {} by {}.",
                        pop,
                        verb,
                        delta.abs()
                    ),
                ));
            }
            last_population_reported = pop;
        }

        let _ = chronicle.flush_tick(tick);

        if alive_count(&agents) == 0 {
            let _ = chronicle.proclaim(&format!(
                "\nSilence falls. No living soul remains. ({})",
                chronicle::describe_season(tick)
            ));
            break;
        }

        if let Some(d) = tick_duration {
            let elapsed = tick_start.elapsed();
            if elapsed < d {
                std::thread::sleep(d - elapsed);
            }
        }
    }

    let _ = chronicle.proclaim(&format!(
        "\nThe chronicle closes. {} souls endure across {} settlements. ({})",
        alive_count(&agents),
        settlements.alive_count(),
        chronicle::describe_season(tick)
    ));

    SimOutcome {
        world,
        agents,
        settlements,
        final_tick: tick,
    }
}

/// Narrate the notable settlements: the largest, plus a struggling one if any.
fn report_settlements(settlements: &Settlements, chronicle: &mut Chronicle, tick: u64) {
    let mut alive: Vec<&settlement::Settlement> =
        settlements.list.iter().filter(|s| s.alive).collect();
    if alive.is_empty() {
        return;
    }
    alive.sort_by_key(|s| std::cmp::Reverse(s.population));
    if let Some(s) = alive.first().copied() {
        let verb = if s.population >= 30 {
            "thrives with"
        } else if s.population >= 10 {
            "holds"
        } else {
            "endures with"
        };
        chronicle.record(Event::new(
            tick,
            format!("{} {} {} souls.", s.name, verb, s.population),
        ));
    }
    if let Some(s) = alive.last().copied() {
        if alive.len() > 1 && s.population <= 4 {
            chronicle.record(Event::new(
                tick,
                format!("{} dwindles to {} inhabitants.", s.name, s.population),
            ));
        }
    }
}

fn describe_major_regions(world: &World) -> String {
    let names = world.major_region_names(3);
    match names.len() {
        0 => String::new(),
        1 => format!("The land holds {}.", names[0]),
        2 => format!("The land holds {} and {}.", names[0], names[1]),
        _ => format!(
            "The land holds {}, {}, and {}.",
            names[0], names[1], names[2]
        ),
    }
}

fn summarize_biomes(world: &World) -> String {
    let mut counts = [0u32; 8];
    for t in &world.tiles {
        let idx = match t.biome {
            Biome::Ocean => 0,
            Biome::Coast => 1,
            Biome::Plains => 2,
            Biome::Forest => 3,
            Biome::Hills => 4,
            Biome::Mountains => 5,
            Biome::Desert => 6,
            Biome::Tundra => 7,
        };
        counts[idx] += 1;
    }
    let total: u32 = counts.iter().sum();
    let pct = |n: u32| (n as f32 / total as f32 * 100.0).round() as u32;
    format!(
        "{}% sea, {}% plains, {}% forest, {}% hills, {}% mountains, {}% desert, {}% tundra, {}% coast",
        pct(counts[0]),
        pct(counts[2]),
        pct(counts[3]),
        pct(counts[4]),
        pct(counts[5]),
        pct(counts[6]),
        pct(counts[7]),
        pct(counts[1]),
    )
}
