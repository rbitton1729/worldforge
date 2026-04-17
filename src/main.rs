use clap::Parser;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::time::{Duration, Instant};
use worldforge::agent::{alive_count, seed_agents, step_agents, Agent};
use worldforge::chronicle::{self, Chronicle, Event};
use worldforge::settlement::{self, update_settlements, Settlements};
use worldforge::world::{self, World};

#[derive(Parser, Debug)]
#[command(
    name = "worldforge",
    about = "A world simulation engine",
    disable_help_flag = true
)]
struct Cli {
    /// Initial population
    #[arg(short = 'n', long = "agents", default_value_t = 200)]
    agents: u32,

    /// RNG seed for reproducibility (random if omitted)
    #[arg(short = 's', long = "seed")]
    seed: Option<u64>,

    /// Simulation speed in ticks/sec
    #[arg(short = 'r', long = "rate", default_value_t = 1.0)]
    rate: f64,

    /// Total ticks to simulate (0 = infinite)
    #[arg(short = 't', long = "ticks", default_value_t = 0)]
    ticks: u64,

    /// Write chronicle to file (default: stdout)
    #[arg(short = 'c', long = "chronicle")]
    chronicle: Option<String>,

    /// Enable TUI mode (not yet implemented)
    #[arg(short = 'g', long = "gui", default_value_t = false)]
    gui: bool,

    /// Map width
    #[arg(short = 'w', long = "width", default_value_t = 80)]
    width: u32,

    /// Map height
    #[arg(short = 'H', long = "height", default_value_t = 40)]
    height: u32,

    /// Disable ANSI color output (also honors NO_COLOR env var)
    #[arg(long = "no-color", default_value_t = false)]
    no_color: bool,

    /// Print help
    #[arg(long = "help", action = clap::ArgAction::Help)]
    help: Option<bool>,
}

fn main() {
    let cli = Cli::parse();

    if cli.gui {
        eprintln!("worldforge: --gui is reserved for Phase 6 and not yet implemented.");
    }

    let seed = cli.seed.unwrap_or_else(|| {
        // Non-deterministic fallback: derive from OS randomness so unseeded runs differ.
        let mut sys_rng = rand::thread_rng();
        sys_rng.r#gen()
    });

    let mut rng = ChaCha8Rng::seed_from_u64(seed);

    let mut chronicle = match cli.chronicle.as_deref() {
        Some(path) => match Chronicle::to_file(path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("worldforge: cannot open chronicle file {}: {}", path, e);
                std::process::exit(1);
            }
        },
        None => Chronicle::to_stdout(),
    };

    if cli.no_color {
        chronicle.set_color(false);
    }

    let _ = chronicle.proclaim(&format!(
        "worldforge — seed {} — {}×{} world — {} souls",
        seed, cli.width, cli.height, cli.agents
    ));

    let mut world = World::generate(cli.width, cli.height, seed);

    // Place agents and describe the world they were born into.
    let mut agents: Vec<Agent> = seed_agents(&world, cli.agents, &mut rng);
    let actual = agents.len();
    if actual < cli.agents as usize {
        let _ = chronicle.proclaim(&format!(
            "The world could only bear {} of the hoped-for {} souls — much of it is sea and stone.",
            actual, cli.agents
        ));
    }

    let biome_summary = summarize_biomes(&world);
    let _ = chronicle.proclaim(&format!(
        "The world takes shape: {}. {} souls draw their first breath.",
        biome_summary, actual
    ));

    let mut settlements = Settlements::new();

    // Emit the prologue under Year 1, Spring.
    chronicle.record(Event::new(
        0,
        "The world awakens. Scattered bands wander the land in search of food."
            .to_string(),
    ));
    let _ = chronicle.flush_tick(0);

    let tick_duration = if cli.rate > 0.0 {
        Some(Duration::from_secs_f64(1.0 / cli.rate))
    } else {
        None
    };

    let mut last_population_reported = actual;
    let mut last_report_year: u64 = 0;

    let mut tick: u64 = 0;
    loop {
        tick += 1;
        let tick_start = Instant::now();

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

        // Settlement population reports twice a year (every 2 seasons = 50 ticks).
        if tick % (chronicle::TICKS_PER_YEAR / 2) == 0 {
            report_settlements(&settlements, &mut chronicle, tick);
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

        // If everyone has perished, close the chronicle.
        if alive_count(&agents) == 0 {
            let _ = chronicle.proclaim(&format!(
                "\nSilence falls. No living soul remains. ({})",
                chronicle::describe_season(tick)
            ));
            break;
        }

        if cli.ticks > 0 && tick >= cli.ticks {
            let _ = chronicle.proclaim(&format!(
                "\nThe chronicle closes. {} souls endure across {} settlements. ({})",
                alive_count(&agents),
                settlements.alive_count(),
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
}

fn report_settlements(settlements: &Settlements, chronicle: &mut Chronicle, tick: u64) {
    // Pick a handful of notable settlements to narrate rather than listing all.
    let mut alive: Vec<&settlement::Settlement> =
        settlements.list.iter().filter(|s| s.alive).collect();
    if alive.is_empty() {
        return;
    }
    alive.sort_by_key(|s| std::cmp::Reverse(s.population));
    // Report the largest and, if there's one struggling, the smallest.
    let largest = alive.first().copied();
    let smallest = alive.iter().rev().copied().next();

    if let Some(s) = largest {
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
    if let Some(s) = smallest {
        if alive.len() > 1 && s.population <= 4 {
            chronicle.record(Event::new(
                tick,
                format!("{} dwindles to {} inhabitants.", s.name, s.population),
            ));
        }
    }
}

fn summarize_biomes(world: &World) -> String {
    use world::Biome;
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
