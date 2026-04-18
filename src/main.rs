use clap::Parser;
use rand::Rng;
use worldforge::chronicle::Chronicle;
use worldforge::{run_simulation, SimConfig};

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

    /// Force ANSI color output on even when not detected as a terminal
    #[arg(long = "color", default_value_t = false)]
    color: bool,

    /// Disable ANSI color output (also honors NO_COLOR env var)
    #[arg(long = "no-color", default_value_t = false)]
    no_color: bool,

    /// Collect per-tick timing stats and print a summary on exit.
    /// Also disables real-time pacing.
    #[arg(long = "profile", default_value_t = false)]
    profile: bool,

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
        let mut sys_rng = rand::thread_rng();
        sys_rng.r#gen()
    });

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
    } else if cli.color {
        chronicle.set_color(true);
    }

    let cfg = SimConfig {
        seed,
        width: cli.width,
        height: cli.height,
        agents: cli.agents,
        ticks: cli.ticks,
        tick_rate: Some(cli.rate),
        profile: cli.profile,
    };

    run_simulation(cfg, &mut chronicle);
}
