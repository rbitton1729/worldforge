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
use world::World;

pub struct SimConfig {
    pub seed: u64,
    pub width: u32,
    pub height: u32,
    pub agents: u32,
    pub ticks: u64,
    pub chronicle_path: Option<String>,
}

impl SimConfig {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            width: 80,
            height: 40,
            agents: 200,
            ticks: 200,
            chronicle_path: None,
        }
    }
}

pub struct SimOutcome {
    pub world: World,
    pub agents: Vec<Agent>,
    pub settlements: Settlements,
    pub final_tick: u64,
}

/// Run a simulation to completion (either all agents dead or cfg.ticks reached).
/// Mirrors the logic in main.rs but without sleep/CLI/stdout-by-default. If
/// `chronicle_path` is None, the chronicle is written to a sink that discards output.
pub fn run_simulation(cfg: SimConfig) -> SimOutcome {
    let mut rng = ChaCha8Rng::seed_from_u64(cfg.seed);

    let mut chronicle = match cfg.chronicle_path.as_deref() {
        Some(path) => Chronicle::to_file(path).expect("chronicle file"),
        None => Chronicle::to_file(
            &std::env::temp_dir()
                .join(format!("worldforge-test-sink-{}.txt", cfg.seed))
                .to_string_lossy(),
        )
        .expect("sink chronicle"),
    };

    let _ = chronicle.proclaim(&format!(
        "worldforge — seed {} — {}×{} world — {} souls",
        cfg.seed, cfg.width, cfg.height, cfg.agents
    ));

    let mut world = World::generate(cfg.width, cfg.height, cfg.seed);
    let mut agents: Vec<Agent> = seed_agents(&world, cfg.agents, &mut rng);
    let mut settlements = Settlements::new();

    let names = world.major_region_names(3);
    if !names.is_empty() {
        let region_clause = match names.len() {
            1 => format!("The land holds {}.", names[0]),
            2 => format!("The land holds {} and {}.", names[0], names[1]),
            _ => format!(
                "The land holds {}, {}, and {}.",
                names[0], names[1], names[2]
            ),
        };
        let _ = chronicle.proclaim(&region_clause);
    }

    chronicle.record(Event::new(
        0,
        "The world awakens. Scattered bands wander the land in search of food.".to_string(),
    ));
    let _ = chronicle.flush_tick(0);

    let mut tick: u64 = 0;
    loop {
        if cfg.ticks > 0 && tick >= cfg.ticks {
            break;
        }
        tick += 1;

        world.regen_food(tick);
        step_agents(
            &mut agents,
            &mut world,
            &mut settlements,
            &mut rng,
            &mut chronicle,
            tick,
        );
        update_settlements(
            &mut settlements,
            &mut agents,
            &world,
            &mut rng,
            &mut chronicle,
            tick,
        );

        chronicle.set_header_stats(alive_count(&agents), settlements.alive_count());

        if tick % (chronicle::TICKS_PER_YEAR / 2) == 0 {
            report_largest_settlement(&settlements, &mut chronicle, tick);
        }

        let _ = chronicle.flush_tick(tick);

        if alive_count(&agents) == 0 {
            let _ = chronicle.proclaim(&format!(
                "\nSilence falls. No living soul remains. ({})",
                chronicle::describe_season(tick)
            ));
            break;
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

fn report_largest_settlement(settlements: &Settlements, chronicle: &mut Chronicle, tick: u64) {
    let mut alive: Vec<&settlement::Settlement> =
        settlements.list.iter().filter(|s| s.alive).collect();
    if alive.is_empty() {
        return;
    }
    alive.sort_by_key(|s| std::cmp::Reverse(s.population));
    if let Some(s) = alive.first() {
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
}
