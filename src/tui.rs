//! Real-time terminal UI for worldforge. Owns its own tick loop so pacing
//! can respond to user input; mirrors the sequence of phases in
//! `run_simulation` but interleaves rendering and keyboard events.

use crate::agent::{alive_count, seed_agents, step_agents, Agent};
use crate::chronicle::{Chronicle, Event, TICKS_PER_YEAR};
use crate::settlement::{update_settlements, Dialects, Settlement, Settlements, Trait};
use crate::world::{Biome, World};
use crate::{SimConfig, SimOutcome};

use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use crossterm::{
    event::{self, KeyCode, KeyEvent, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame, Terminal,
};

use std::collections::{HashMap, VecDeque};
use std::io::{self, Stdout};
use std::time::{Duration, Instant};

/// Tick-rate presets cycled by [+]/[-]. 0 = flat-out.
const SPEEDS: &[u32] = &[1, 5, 10, 30, 60, 0];
const DEFAULT_SPEED_IDX: usize = 2; // 10 tps
const EVENT_LOG_CAP: usize = 500;
/// How long a settlement stays tinted red after a raid it was involved in.
/// Long enough to register at slow speeds; short enough to feel like a flash
/// at 30+ tps.
const RAID_FLASH_TICKS: u64 = 8;

// ---- trait / decor colors ----
const C_MARKER_DEFAULT: Color = Color::Rgb(255, 240, 180);
const C_MARKER_MILITANT: Color = Color::Rgb(235, 105, 80);
const C_MARKER_MERCANTILE: Color = Color::Rgb(250, 210, 100);
const C_MARKER_DEPLETED: Color = Color::Rgb(180, 180, 170);
const C_MARKER_DEAD: Color = Color::Rgb(100, 100, 110);
const C_FLASH_BG: Color = Color::Rgb(200, 40, 40);
const C_ROUTE_ALLIED: Color = Color::Rgb(240, 205, 110);
const C_ROUTE_DECLARED: Color = Color::Rgb(160, 140, 105);
const C_FEUD: Color = Color::Rgb(180, 70, 70);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Zoom {
    Out,    // fit full world (scale ≥ 1)
    Normal, // 1 term col per tile, half-block vertical
    In,     // 2×2 term cells per tile
}

impl Zoom {
    fn next(self) -> Self {
        match self {
            Zoom::Out => Zoom::Normal,
            Zoom::Normal => Zoom::In,
            Zoom::In => Zoom::Out,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Zoom::Out => "out",
            Zoom::Normal => "1:1",
            Zoom::In => "in",
        }
    }
}

pub fn run(cfg: SimConfig, chronicle: &mut Chronicle) -> io::Result<SimOutcome> {
    let mut term = setup_terminal()?;
    let result = run_inner(&mut term, cfg, chronicle);
    // Always restore the terminal, even on error — an abandoned alt-screen
    // leaves the user's shell wedged in raw mode.
    let _ = restore_terminal(&mut term);
    result
}

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    // Intentionally no EnableMouseCapture: the TUI doesn't use mouse events,
    // and capture would prevent the user from selecting text in their
    // terminal (a regression users notice immediately).
    enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(out))
}

fn restore_terminal(term: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(term.backend_mut(), LeaveAlternateScreen)?;
    term.show_cursor()?;
    Ok(())
}

struct TuiState {
    event_log: VecDeque<(u64, String)>,
    speed_idx: usize,
    paused: bool,
    show_chronicle: bool,
    show_help: bool,
    zoom: Zoom,
    cam_col: i32,
    cam_row: i32,
    chronicle_scroll: u16,
    /// sid → tick at which the red flash ends
    flash_until: HashMap<u32, u64>,
    /// sid → last-seen `raids_done` and `raids_suffered` (for flash triggers)
    prev_raids_done: HashMap<u32, u32>,
    prev_raids_suffered: HashMap<u32, u32>,
    /// Total pop at the start of the current year — used to derive the
    /// yearly trend arrow shown in the stats bar.
    year_start_pop: u32,
    /// Signed delta from last completed year. 0 before any year closes.
    last_year_delta: i32,
}

fn season_idx(tick: u64) -> usize {
    ((tick % TICKS_PER_YEAR) / (TICKS_PER_YEAR / 4)) as usize
}

fn run_inner(
    term: &mut Terminal<CrosstermBackend<Stdout>>,
    cfg: SimConfig,
    chronicle: &mut Chronicle,
) -> io::Result<SimOutcome> {
    // --- sim init (mirrors run_simulation, minus the chronicle prologue: the
    // TUI shows the world visually, so a prose prologue would clutter the feed).
    let mut rng = ChaCha8Rng::seed_from_u64(cfg.seed);
    let mut world = World::generate(cfg.width, cfg.height, cfg.seed);
    let mut agents: Vec<Agent> = seed_agents(&world, cfg.agents, &mut rng);
    let mut settlements = Settlements::new();
    settlements.set_dialects(Dialects::generate(&world, cfg.seed));
    chronicle.record(Event::new(
        0,
        "The world awakens. Scattered bands wander the land in search of food.".to_string(),
    ));
    let _ = chronicle.flush_tick(0);

    let mut ui = TuiState {
        event_log: VecDeque::with_capacity(EVENT_LOG_CAP),
        speed_idx: DEFAULT_SPEED_IDX,
        paused: false,
        show_chronicle: false,
        show_help: false,
        zoom: Zoom::Normal,
        cam_col: cfg.width as i32 / 2,
        cam_row: cfg.height as i32 / 2,
        chronicle_scroll: 0,
        flash_until: HashMap::new(),
        prev_raids_done: HashMap::new(),
        prev_raids_suffered: HashMap::new(),
        year_start_pop: alive_count(&agents) as u32,
        last_year_delta: 0,
    };

    let mut tick: u64 = 0;
    let mut next_tick_due = Instant::now();
    let mut last_population_reported = alive_count(&agents);
    let mut last_report_year: u64 = 0;
    let mut sim_over = false;

    loop {
        // --- input (non-blocking poll)
        if event::poll(Duration::from_millis(0))? {
            if let event::Event::Key(key) = event::read()? {
                if handle_key(key, &mut ui, &settlements, &world) {
                    break;
                }
            }
        }

        // --- tick (pacing-controlled)
        let now = Instant::now();
        let should_tick = !ui.paused && !sim_over && now >= next_tick_due;
        if should_tick {
            tick += 1;
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

            // Detect raid activity — any raids_done or raids_suffered bump
            // since last tick triggers a short red flash on that settlement.
            update_raid_flashes(&mut ui, &settlements, tick);

            // Yearly population delta — same condition as run_simulation.
            let year = tick / TICKS_PER_YEAR + 1;
            if tick % TICKS_PER_YEAR == 0 && year != last_report_year {
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
                // TUI trend: compare to pop at start of the year that just closed.
                ui.last_year_delta = pop as i32 - ui.year_start_pop as i32;
                ui.year_start_pop = pop as u32;
            }

            // Snapshot events into the UI feed before flush clears them.
            for ev in chronicle.peek_pending() {
                ui.event_log.push_back((ev.tick, ev.text.clone()));
            }
            while ui.event_log.len() > EVENT_LOG_CAP {
                ui.event_log.pop_front();
            }

            let _ = chronicle.flush_tick(tick);

            if alive_count(&agents) == 0 || (cfg.ticks > 0 && tick >= cfg.ticks) {
                sim_over = true;
            }

            let tps = SPEEDS[ui.speed_idx];
            next_tick_due = if tps == 0 {
                now
            } else {
                now + Duration::from_secs_f64(1.0 / tps as f64)
            };
        }

        // --- draw
        term.draw(|f| draw(f, &world, &settlements, &agents, tick, sim_over, &ui))?;

        // --- idle: avoid burning CPU when paused or between scheduled ticks.
        if !should_tick {
            let cap = Duration::from_millis(16); // ~60 fps input responsiveness
            let wait = next_tick_due.saturating_duration_since(Instant::now()).min(cap);
            if ui.paused || sim_over {
                std::thread::sleep(cap);
            } else if !wait.is_zero() {
                std::thread::sleep(wait);
            }
        }
    }

    Ok(SimOutcome {
        world,
        agents,
        settlements,
        final_tick: tick,
    })
}

/// Compare this tick's raid counters to the last-seen values. Any delta means
/// the settlement was involved in a raid during this tick — tint it red for
/// `RAID_FLASH_TICKS` upcoming ticks so the user sees the action.
fn update_raid_flashes(ui: &mut TuiState, settlements: &Settlements, tick: u64) {
    for s in &settlements.list {
        let prev_done = ui.prev_raids_done.get(&s.id).copied().unwrap_or(0);
        let prev_suff = ui.prev_raids_suffered.get(&s.id).copied().unwrap_or(0);
        if s.raids_done > prev_done || s.raids_suffered > prev_suff {
            ui.flash_until.insert(s.id, tick + RAID_FLASH_TICKS);
        }
        ui.prev_raids_done.insert(s.id, s.raids_done);
        ui.prev_raids_suffered.insert(s.id, s.raids_suffered);
    }
    // Drop expired entries so the map doesn't slowly bloat.
    ui.flash_until.retain(|_, until| *until >= tick);
}

/// Handle one key event. Returns true if the user asked to quit.
fn handle_key(key: KeyEvent, ui: &mut TuiState, settlements: &Settlements, world: &World) -> bool {
    if key.kind == KeyEventKind::Release {
        return false;
    }
    // Pan step: bigger when zoomed out (each char covers more tiles),
    // smaller when zoomed in.
    let pan = match ui.zoom {
        Zoom::Out => 4,
        Zoom::Normal => 2,
        Zoom::In => 1,
    };
    let in_chronicle = ui.show_chronicle;
    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => return true,
        KeyCode::Char(' ') => ui.paused = !ui.paused,
        KeyCode::Char('+') | KeyCode::Char('=') => {
            ui.speed_idx = (ui.speed_idx + 1).min(SPEEDS.len() - 1);
        }
        KeyCode::Char('-') | KeyCode::Char('_') => {
            ui.speed_idx = ui.speed_idx.saturating_sub(1);
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            ui.show_chronicle = !ui.show_chronicle;
            ui.chronicle_scroll = 0;
        }
        KeyCode::Char('?') | KeyCode::Char('h') | KeyCode::Char('H') => {
            ui.show_help = !ui.show_help;
        }
        KeyCode::Char('c') | KeyCode::Char('C') => {
            // Recenter camera on the most-populated settlement, or world center.
            if let Some((c, r)) = most_populated(settlements) {
                ui.cam_col = c;
                ui.cam_row = r;
            } else {
                ui.cam_col = world.width as i32 / 2;
                ui.cam_row = world.height as i32 / 2;
            }
        }
        KeyCode::Char('z') | KeyCode::Char('Z') => {
            ui.zoom = ui.zoom.next();
            // Entering zoomed-in: recenter on the most-populated settlement
            // so the user has something interesting in view by default.
            if ui.zoom == Zoom::In {
                if let Some((c, r)) = most_populated(settlements) {
                    ui.cam_col = c;
                    ui.cam_row = r;
                } else {
                    ui.cam_col = world.width as i32 / 2;
                    ui.cam_row = world.height as i32 / 2;
                }
            }
        }
        KeyCode::Left => {
            if !in_chronicle {
                ui.cam_col -= pan;
            }
        }
        KeyCode::Right => {
            if !in_chronicle {
                ui.cam_col += pan;
            }
        }
        KeyCode::Up => {
            if in_chronicle {
                ui.chronicle_scroll = ui.chronicle_scroll.saturating_sub(1);
            } else {
                ui.cam_row -= pan;
            }
        }
        KeyCode::Down => {
            if in_chronicle {
                ui.chronicle_scroll = ui.chronicle_scroll.saturating_add(1);
            } else {
                ui.cam_row += pan;
            }
        }
        KeyCode::PageUp => {
            if in_chronicle {
                ui.chronicle_scroll = ui.chronicle_scroll.saturating_sub(10);
            }
        }
        KeyCode::PageDown => {
            if in_chronicle {
                ui.chronicle_scroll = ui.chronicle_scroll.saturating_add(10);
            }
        }
        _ => {}
    }
    // Keep the camera in a sane range — never more than a map's width/height
    // outside the world, so panning off-screen recovers quickly.
    let margin_c = world.width as i32;
    let margin_r = world.height as i32;
    ui.cam_col = ui.cam_col.clamp(-margin_c, 2 * margin_c);
    ui.cam_row = ui.cam_row.clamp(-margin_r, 2 * margin_r);
    false
}

fn most_populated(settlements: &Settlements) -> Option<(i32, i32)> {
    settlements
        .list
        .iter()
        .filter(|s| s.alive)
        .max_by_key(|s| s.population)
        .map(|s| (s.col, s.row))
}

// ---------- drawing ----------

fn draw(
    f: &mut Frame,
    world: &World,
    settlements: &Settlements,
    agents: &[Agent],
    tick: u64,
    sim_over: bool,
    ui: &TuiState,
) {
    let outer = f.area();

    // Reserve 2 lines for the stats bar at the bottom and ~22% of remaining
    // height for the events panel. The map gets whatever's left.
    let stats_h: u16 = 2;
    let remaining = outer.height.saturating_sub(stats_h);
    let events_h: u16 = (remaining as u32 * 22 / 100).clamp(5, 15) as u16;

    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Length(events_h),
            Constraint::Length(stats_h),
        ])
        .split(outer);
    let map_area = v[0];
    let events_area = v[1];
    let stats_area = v[2];

    if ui.show_chronicle {
        draw_chronicle(f, map_area, &ui.event_log, ui.chronicle_scroll);
    } else {
        draw_map(f, map_area, world, settlements, agents, tick, ui);
    }
    draw_events(f, events_area, &ui.event_log);
    draw_stats_bar(f, stats_area, world, settlements, agents, tick, sim_over, ui);

    if ui.show_help {
        draw_help_overlay(f, outer);
    }
}

fn speed_label(idx: usize) -> String {
    match SPEEDS[idx] {
        0 => "max".to_string(),
        n => format!("{}", n),
    }
}

fn trend_arrow(delta: i32) -> (&'static str, Color) {
    if delta > 0 {
        ("▲", Color::Rgb(120, 220, 120))
    } else if delta < 0 {
        ("▼", Color::Rgb(230, 120, 120))
    } else {
        ("·", Color::Rgb(160, 160, 160))
    }
}

fn draw_stats_bar(
    f: &mut Frame,
    area: Rect,
    world: &World,
    settlements: &Settlements,
    agents: &[Agent],
    tick: u64,
    sim_over: bool,
    ui: &TuiState,
) {
    let pop = alive_count(agents);
    let settle_count = settlements.alive_count();
    let custom_count: usize = settlements
        .list
        .iter()
        .filter(|s| s.alive)
        .map(|s| s.customs.len())
        .sum();
    let legend_count = agents.iter().filter(|a| a.alive && a.epithet.is_some()).count();
    let year = tick / TICKS_PER_YEAR + 1;
    let season = ["Spring", "Summer", "Autumn", "Winter"][season_idx(tick)];
    let status = if sim_over {
        "ended"
    } else if ui.paused {
        "paused"
    } else {
        "running"
    };
    let (arrow, arrow_color) = trend_arrow(ui.last_year_delta);

    let bg = Color::Rgb(32, 34, 44);
    let fg = Color::Rgb(220, 220, 220);
    let dim = Color::Rgb(140, 140, 150);
    let accent = Color::Rgb(255, 220, 130);

    let sep = || Span::styled(" │ ", Style::default().fg(dim).bg(bg));

    let delta_str = if ui.last_year_delta == 0 {
        String::from(" (—)")
    } else {
        format!(" ({:+})", ui.last_year_delta)
    };

    let stats_line = Line::from(vec![
        Span::styled(" Year ", Style::default().fg(dim).bg(bg)),
        Span::styled(
            format!("{} ({})", year, season),
            Style::default().fg(accent).bg(bg).add_modifier(Modifier::BOLD),
        ),
        sep(),
        Span::styled("Pop ", Style::default().fg(dim).bg(bg)),
        Span::styled(format!("{}", pop), Style::default().fg(Color::LightGreen).bg(bg)),
        Span::styled(" ", Style::default().bg(bg)),
        Span::styled(arrow, Style::default().fg(arrow_color).bg(bg)),
        Span::styled(delta_str, Style::default().fg(dim).bg(bg)),
        sep(),
        Span::styled("Settlements ", Style::default().fg(dim).bg(bg)),
        Span::styled(format!("{}", settle_count), Style::default().fg(fg).bg(bg)),
        sep(),
        Span::styled("Customs ", Style::default().fg(dim).bg(bg)),
        Span::styled(
            format!("{}", custom_count),
            Style::default().fg(Color::Rgb(220, 180, 240)).bg(bg),
        ),
        sep(),
        Span::styled("Legends ", Style::default().fg(dim).bg(bg)),
        Span::styled(
            format!("{}", legend_count),
            Style::default().fg(accent).bg(bg).add_modifier(Modifier::BOLD),
        ),
        sep(),
        Span::styled("Speed ", Style::default().fg(dim).bg(bg)),
        Span::styled(
            format!("{} tps", speed_label(ui.speed_idx)),
            Style::default().fg(fg).bg(bg),
        ),
        sep(),
        Span::styled("Zoom ", Style::default().fg(dim).bg(bg)),
        Span::styled(ui.zoom.label(), Style::default().fg(fg).bg(bg)),
        sep(),
        Span::styled("Climate ", Style::default().fg(dim).bg(bg)),
        Span::styled(
            format!("{:+.2}", world.climate_drift),
            Style::default().fg(fg).bg(bg),
        ),
        sep(),
        Span::styled(status, Style::default().fg(accent).bg(bg)),
    ]);

    let keys_line = Line::from(vec![Span::styled(
        " [q]uit  [space]pause  [+/-]speed  [z]oom  [c]enter  [r]chronicle  [?]help  [←↑↓→]pan ",
        Style::default().fg(fg).bg(bg),
    )]);

    f.render_widget(
        Paragraph::new(vec![stats_line, keys_line]).style(Style::default().bg(bg)),
        area,
    );
}

fn draw_events(f: &mut Frame, area: Rect, log: &VecDeque<(u64, String)>) {
    let block = Block::default().borders(Borders::ALL).title(" Events ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let visible_rows = inner.height as usize;
    let inner_w = inner.width as usize;
    if visible_rows == 0 || inner_w == 0 {
        return;
    }

    let lines: Vec<Line> = log
        .iter()
        .rev()
        .take(visible_rows)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .enumerate()
        .map(|(i, (t, text))| {
            let depth = visible_rows.saturating_sub(i + 1);
            format_event_line(*t, text, inner_w, depth)
        })
        .collect();

    f.render_widget(Paragraph::new(lines), inner);
}

/// `depth` is how many lines above the newest the entry sits (0 = newest).
/// Older lines are dimmed so the eye tracks the bottom as the live feed.
fn format_event_line(tick: u64, text: &str, max_width: usize, depth: usize) -> Line<'static> {
    let year = tick / TICKS_PER_YEAR + 1;
    let season = ["Sp", "Su", "Au", "Wi"][season_idx(tick)];
    let prefix = format!("Y{:<4} {} ", year, season);
    let prefix_len = prefix.chars().count();
    let budget = max_width.saturating_sub(prefix_len);
    let body: String = if text.chars().count() > budget {
        let mut s: String = text.chars().take(budget.saturating_sub(1)).collect();
        s.push('…');
        s
    } else {
        text.to_string()
    };
    let body_color = if depth == 0 {
        // Highlight dramatic lines (the chronicle uses *** markers for them).
        if body.starts_with("***") {
            Color::Rgb(255, 200, 120)
        } else {
            Color::Rgb(230, 230, 230)
        }
    } else if depth <= 2 {
        Color::Rgb(200, 200, 200)
    } else if depth <= 6 {
        Color::Rgb(160, 160, 160)
    } else {
        Color::Rgb(120, 120, 120)
    };
    Line::from(vec![
        Span::styled(prefix, Style::default().fg(Color::Rgb(105, 105, 115))),
        Span::styled(body, Style::default().fg(body_color)),
    ])
}

fn biome_color(b: Biome) -> Color {
    match b {
        Biome::Ocean => Color::Rgb(18, 42, 90),
        Biome::Coast => Color::Rgb(80, 150, 190),
        Biome::Plains => Color::Rgb(120, 170, 70),
        Biome::Forest => Color::Rgb(35, 85, 45),
        Biome::Hills => Color::Rgb(160, 125, 60),
        Biome::Mountains => Color::Rgb(155, 150, 155),
        Biome::Desert => Color::Rgb(220, 190, 100),
        Biome::Tundra => Color::Rgb(190, 205, 220),
    }
}

fn river_color(b: Biome) -> Color {
    if b == Biome::Ocean {
        biome_color(b)
    } else {
        Color::Rgb(95, 185, 235)
    }
}

fn tile_color(world: &World, col: i32, row: i32) -> Color {
    let Some(tile) = world.tile(col, row) else {
        return Color::Black;
    };
    if tile.river > 0 {
        river_color(tile.biome)
    } else {
        biome_color(tile.biome)
    }
}

fn settlement_fg(s: &Settlement) -> Color {
    match s.trait_kind {
        Some(Trait::Militant) => C_MARKER_MILITANT,
        Some(Trait::Mercantile) => C_MARKER_MERCANTILE,
        None if s.land_depleted => C_MARKER_DEPLETED,
        None => C_MARKER_DEFAULT,
    }
}

fn settlement_marker_char(s: &Settlement) -> char {
    // Named settlement marker, uppercased. `legend_fifty` settlements get a
    // bold star to stand out visually; same-letter name collisions disambiguate
    // via the character list.
    s.name.chars().next().unwrap_or('#').to_ascii_uppercase()
}

// ---- route plotting ----

/// Emit every cell (x, y) along the Bresenham line from (x0, y0) to (x1, y1).
fn bresenham_line<F: FnMut(i32, i32)>(mut x0: i32, mut y0: i32, x1: i32, y1: i32, mut put: F) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        put(x0, y0);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

struct RouteCell {
    fg: Color,
    ch: char,
}

fn draw_map(
    f: &mut Frame,
    area: Rect,
    world: &World,
    settlements: &Settlements,
    agents: &[Agent],
    tick: u64,
    ui: &TuiState,
) {
    let title = match ui.zoom {
        Zoom::Out => format!(" World  {}×{}  (zoomed out) ", world.width, world.height),
        Zoom::Normal => format!(" World  {}×{} ", world.width, world.height),
        Zoom::In => format!(" World  {}×{}  (zoomed in) ", world.width, world.height),
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    // Per-settlement count of living agents that have earned an epithet.
    // Used by zoom-in/zoom-normal to mark culturally notable settlements.
    let mut legend_counts: HashMap<u32, u32> = HashMap::new();
    for a in agents {
        if !a.alive || a.epithet.is_none() {
            continue;
        }
        if let Some(sid) = a.settlement {
            *legend_counts.entry(sid).or_insert(0) += 1;
        }
    }

    match ui.zoom {
        Zoom::Normal => {
            draw_map_halfblock(f, inner, world, settlements, &legend_counts, tick, ui, 1, false)
        }
        Zoom::Out => {
            let cols = inner.width as u32;
            let halfrows = inner.height as u32 * 2;
            let sx = world.width.div_ceil(cols.max(1));
            let sy = world.height.div_ceil(halfrows.max(1));
            let scale = sx.max(sy).max(1);
            draw_map_halfblock(
                f,
                inner,
                world,
                settlements,
                &legend_counts,
                tick,
                ui,
                scale,
                true,
            );
        }
        Zoom::In => draw_map_zoomed_in(f, inner, world, settlements, &legend_counts, tick, ui),
    }
}

/// Convert a world (col, row) to (term_col, term_row) in the half-block viewport.
/// Returns None if the position falls outside the visible rectangle.
fn world_to_term(
    col: i32,
    row: i32,
    start_col: i32,
    start_row: i32,
    scale_x: i32,
    scale_y: i32,
    cols: i32,
    rows: i32,
) -> Option<(i32, i32)> {
    let dc = col - start_col;
    let dr = row - start_row;
    if dc < 0 || dr < 0 {
        return None;
    }
    let tc = dc / scale_x;
    let tr = (dr / scale_y) / 2;
    if tc < 0 || tc >= cols || tr < 0 || tr >= rows {
        return None;
    }
    Some((tc, tr))
}

/// Half-block renderer. Each terminal char represents `scale` map cols wide
/// and `2*scale` map rows tall (top half + bottom half, each covering `scale`
/// map rows). `fit_world=true` centers the world within the viewport; otherwise
/// the viewport is centered on the camera.
fn draw_map_halfblock(
    f: &mut Frame,
    inner: Rect,
    world: &World,
    settlements: &Settlements,
    legend_counts: &HashMap<u32, u32>,
    tick: u64,
    ui: &TuiState,
    scale: u32,
    fit_world: bool,
) {
    let cols = inner.width as i32;
    let rows = inner.height as i32;
    let s = scale as i32;

    let (start_col, start_row) = if fit_world {
        let cov_cols = cols * s;
        let cov_rows = rows * 2 * s;
        (
            (world.width as i32 - cov_cols) / 2,
            (world.height as i32 - cov_rows) / 2,
        )
    } else {
        let visible_cols = cols * s;
        let visible_rows = rows * 2 * s;
        (ui.cam_col - visible_cols / 2, ui.cam_row - visible_rows / 2)
    };

    // -- route overlay (declared + allied trade routes, plus blood feuds)
    // Drawn first so settlement markers render on top.
    let mut routes: HashMap<(i32, i32), RouteCell> = HashMap::new();
    plot_routes(
        &mut routes,
        settlements,
        |c, r| world_to_term(c, r, start_col, start_row, s, s, cols, rows),
    );

    // -- settlement markers (live + ruins), indexed by (term_col, term_row).
    // Each cell is one half-block char cell; a settlement wins the whole cell
    // (loses the other half's biome) so the marker is actually legible.
    let mut markers: HashMap<(i32, i32), Marker> = HashMap::new();
    for st in &settlements.list {
        let Some((tc, tr)) = world_to_term(st.col, st.row, start_col, start_row, s, s, cols, rows)
        else {
            continue;
        };
        let tile_bg = tile_color(world, st.col, st.row);
        if !st.alive {
            // Ruin: only render if nothing already there (a live settlement
            // later founded on the same term-cell takes priority).
            markers.entry((tc, tr)).or_insert(Marker {
                ch: '·',
                fg: C_MARKER_DEAD,
                bg: tile_bg,
                bold: false,
                dim: true,
                underlined: false,
                priority: 0,
            });
            continue;
        }
        let flashing = ui
            .flash_until
            .get(&st.id)
            .map_or(false, |until| *until >= tick);
        let bg = if flashing { C_FLASH_BG } else { tile_bg };
        // Culturally notable = has customs or harbors a living legend agent.
        // Underlined marker is the subtlest signal that fits in a half-block cell.
        let notable = !st.customs.is_empty() || legend_counts.get(&st.id).copied().unwrap_or(0) > 0;
        let marker = Marker {
            ch: settlement_marker_char(st),
            fg: settlement_fg(st),
            bg,
            bold: true,
            dim: false,
            underlined: notable,
            priority: st.population.saturating_add(1),
        };
        // Larger settlement wins cell collision.
        markers
            .entry((tc, tr))
            .and_modify(|existing| {
                if marker.priority > existing.priority {
                    *existing = marker;
                }
            })
            .or_insert(marker);
    }

    let half_offset = s / 2;
    let mut lines: Vec<Line> = Vec::with_capacity(rows as usize);
    for term_row in 0..rows {
        let top_map_row = start_row + term_row * 2 * s + half_offset;
        let bot_map_row = top_map_row + s;
        let mut spans: Vec<Span> = Vec::with_capacity(cols as usize);
        for term_col in 0..cols {
            let map_col = start_col + term_col * s + half_offset;
            if let Some(m) = markers.get(&(term_col, term_row)) {
                let mut style = Style::default().fg(m.fg).bg(m.bg);
                if m.bold {
                    style = style.add_modifier(Modifier::BOLD);
                }
                if m.dim {
                    style = style.add_modifier(Modifier::DIM);
                }
                if m.underlined {
                    style = style.add_modifier(Modifier::UNDERLINED);
                }
                spans.push(Span::styled(String::from(m.ch), style));
                continue;
            }
            let top_color = tile_color(world, map_col, top_map_row);
            let bot_color = tile_color(world, map_col, bot_map_row);
            if let Some(rc) = routes.get(&(term_col, term_row)) {
                // Route dot: use the top-half biome as bg so it feels embedded.
                spans.push(Span::styled(
                    String::from(rc.ch),
                    Style::default().fg(rc.fg).bg(top_color),
                ));
            } else {
                spans.push(Span::styled(
                    "▀".to_string(),
                    Style::default().fg(top_color).bg(bot_color),
                ));
            }
        }
        lines.push(Line::from(spans));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

#[derive(Clone, Copy)]
struct Marker {
    ch: char,
    fg: Color,
    bg: Color,
    bold: bool,
    dim: bool,
    /// Settlement has customs and/or living legend agents — rendered as
    /// underline on the marker glyph so the eye can spot notable places.
    underlined: bool,
    /// Used to resolve collisions when several settlements fall in one cell
    /// at high zoom-out. Higher = wins. Live settlements use population + 1;
    /// ruins use 0 so any living settlement overrides a ruin.
    priority: u32,
}

/// Plot route cells into `out` using a `to_term` mapper from world (col, row)
/// to terminal (tc, tr). Cells are keyed by term-cell so multiple routes
/// passing through the same cell don't stack.
fn plot_routes<F: Fn(i32, i32) -> Option<(i32, i32)>>(
    out: &mut HashMap<(i32, i32), RouteCell>,
    settlements: &Settlements,
    to_term: F,
) {
    for s in &settlements.list {
        if !s.alive {
            continue;
        }
        let Some(a) = to_term(s.col, s.row) else {
            continue;
        };
        // Trade routes: draw each pair once (s.id < other.id) so the same
        // line isn't painted twice.
        for r in &s.routes {
            if r.other_id <= s.id || !r.declared {
                continue;
            }
            let Some(other) = settlements.list.iter().find(|o| o.id == r.other_id && o.alive)
            else {
                continue;
            };
            let Some(b) = to_term(other.col, other.row) else {
                continue;
            };
            let (fg, ch) = if r.allied {
                (C_ROUTE_ALLIED, '•')
            } else {
                (C_ROUTE_DECLARED, '·')
            };
            let (priority_allied, priority_feud) = (r.allied, false);
            paint_line(out, a, b, fg, ch, priority_allied, priority_feud);
        }
        // Blood feuds: red hatched line (overrides declared routes when both exist).
        for e in &s.enmities {
            if !e.blood_feud || e.other_id <= s.id {
                continue;
            }
            let Some(other) = settlements.list.iter().find(|o| o.id == e.other_id && o.alive)
            else {
                continue;
            };
            let Some(b) = to_term(other.col, other.row) else {
                continue;
            };
            paint_line(out, a, b, C_FEUD, '╳', false, true);
        }
    }
}

fn paint_line(
    out: &mut HashMap<(i32, i32), RouteCell>,
    a: (i32, i32),
    b: (i32, i32),
    fg: Color,
    ch: char,
    priority_allied: bool,
    priority_feud: bool,
) {
    bresenham_line(a.0, a.1, b.0, b.1, |x, y| {
        // Don't clobber endpoints (the settlement markers will render there).
        if (x, y) == a || (x, y) == b {
            return;
        }
        let new_rank = route_rank(priority_allied, priority_feud);
        match out.get(&(x, y)) {
            Some(existing) if route_rank_of_char(existing.ch) >= new_rank => {}
            _ => {
                out.insert((x, y), RouteCell { fg, ch });
            }
        }
    });
}

fn route_rank(allied: bool, feud: bool) -> u8 {
    if feud {
        3
    } else if allied {
        2
    } else {
        1
    }
}

fn route_rank_of_char(ch: char) -> u8 {
    match ch {
        '╳' => 3,
        '•' => 2,
        '·' => 1,
        _ => 0,
    }
}

/// Zoomed-in renderer: each world tile occupies 2 terminal columns × 1 terminal
/// row (using half-block characters for vertical sub-tiles). This preserves the
/// same aspect-ratio correction as the normal view so tiles stay rectangular.
fn draw_map_zoomed_in(
    f: &mut Frame,
    inner: Rect,
    world: &World,
    settlements: &Settlements,
    _legend_counts: &HashMap<u32, u32>,
    tick: u64,
    ui: &TuiState,
) {
    let tile_w: i32 = 2; // terminal columns per tile
    let tiles_cols = (inner.width as i32) / tile_w;
    let tiles_rows = inner.height as i32; // each term row = 2 world rows via ▀
    if tiles_cols <= 0 || tiles_rows <= 0 {
        return;
    }

    let left = ui.cam_col - tiles_cols / 2;
    let top = ui.cam_row - tiles_rows; // each term row covers 2 world rows

    // Routes at tile-cell granularity; the dot lands in the tile's top-left
    // sub-cell (mirrors where settlement markers land).
    let mut routes_tile: HashMap<(i32, i32), RouteCell> = HashMap::new();
    plot_routes(&mut routes_tile, settlements, |c, r| {
        let tc = (c - left) / tile_w;
        let tr = (r - top) / 2;
        if tc < 0 || tc >= tiles_cols || tr < 0 || tr >= tiles_rows {
            None
        } else {
            Some((tc, tr))
        }
    });

    // Settlement markers indexed at tile granularity (not sub-cell).
    struct Mk {
        ch: char,
        fg: Color,
        bg_override: Option<Color>,
        bold: bool,
        dim: bool,
    }
    let mut marker_tile: HashMap<(i32, i32), Mk> = HashMap::new();
    for st in &settlements.list {
        let tc = (st.col - left) / tile_w;
        let tr = (st.row - top) / 2;
        if tc < 0 || tc >= tiles_cols || tr < 0 || tr >= tiles_rows {
            continue;
        }
        if !st.alive {
            marker_tile.entry((tc, tr)).or_insert(Mk {
                ch: '·',
                fg: C_MARKER_DEAD,
                bg_override: None,
                bold: false,
                dim: true,
            });
            continue;
        }
        let flashing = ui
            .flash_until
            .get(&st.id)
            .map_or(false, |until| *until >= tick);
        marker_tile.insert(
            (tc, tr),
            Mk {
                ch: settlement_marker_char(st),
                fg: settlement_fg(st),
                bg_override: if flashing { Some(C_FLASH_BG) } else { None },
                bold: true,
                dim: false,
            },
        );
    }

    let mut lines: Vec<Line> = Vec::with_capacity(tiles_rows as usize);
    for term_row in 0..tiles_rows {
        let top_map_row = top + term_row * 2;
        let bot_map_row = top_map_row + 1;
        let mut spans: Vec<Span> = Vec::with_capacity((tiles_cols * tile_w) as usize);
        for tile_col_off in 0..tiles_cols {
            let tile_col = left + tile_col_off * tile_w;
            let top_color = tile_color(world, tile_col, top_map_row);
            let bot_color = tile_color(world, tile_col, bot_map_row);
            let tile_key = (tile_col_off, term_row);
            if let Some(m) = marker_tile.get(&tile_key) {
                let bg = m.bg_override.unwrap_or(bot_color);
                let mut style = Style::default().fg(m.fg).bg(bg);
                if m.bold {
                    style = style.add_modifier(Modifier::BOLD);
                }
                if m.dim {
                    style = style.add_modifier(Modifier::DIM);
                }
                // Marker in first col of the tile, blank fill in second col
                spans.push(Span::styled(String::from(m.ch), style));
                if let Some(flash_bg) = m.bg_override {
                    spans.push(Span::styled(
                        " ".to_string(),
                        Style::default().bg(flash_bg),
                    ));
                } else {
                    spans.push(Span::styled(
                        "▀".to_string(),
                        Style::default().fg(top_color).bg(bot_color),
                    ));
                }
                continue;
            }
            if let Some(rc) = routes_tile.get(&tile_key) {
                spans.push(Span::styled(
                    String::from(rc.ch),
                    Style::default().fg(rc.fg).bg(top_color),
                ));
                spans.push(Span::styled(
                    "▀".to_string(),
                    Style::default().fg(top_color).bg(bot_color),
                ));
            } else {
                spans.push(Span::styled(
                    "▀".to_string(),
                    Style::default().fg(top_color).bg(bot_color),
                ));
                spans.push(Span::styled(
                    "▀".to_string(),
                    Style::default().fg(top_color).bg(bot_color),
                ));
            }
        }
        lines.push(Line::from(spans));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_chronicle(f: &mut Frame, area: Rect, log: &VecDeque<(u64, String)>, scroll: u16) {
    let block = Block::default().borders(Borders::ALL).title(" Chronicle ");
    let lines: Vec<Line> = log
        .iter()
        .map(|(t, text)| {
            let year = t / TICKS_PER_YEAR + 1;
            let season = ["Sp", "Su", "Au", "Wi"][season_idx(*t)];
            let body_color = if text.starts_with("***") {
                Color::Rgb(255, 200, 120)
            } else {
                Color::Rgb(220, 220, 220)
            };
            Line::from(vec![
                Span::styled(
                    format!("Y{:<5} {} ", year, season),
                    Style::default().fg(Color::Rgb(110, 110, 120)),
                ),
                Span::styled(text.clone(), Style::default().fg(body_color)),
            ])
        })
        .collect();
    let p = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: true })
        .scroll((scroll, 0));
    f.render_widget(p, area);
}

fn draw_help_overlay(f: &mut Frame, full: Rect) {
    let lines = vec![
        Line::from(Span::styled(
            " worldforge — keybindings ",
            Style::default()
                .fg(Color::Rgb(255, 220, 130))
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(" q / Esc     quit"),
        Line::from(" space       pause / resume"),
        Line::from(" + / -       faster / slower"),
        Line::from(" z           cycle zoom (out → 1:1 → in)"),
        Line::from(" c           center camera on largest settlement"),
        Line::from(" ← ↑ ↓ →     pan camera"),
        Line::from(" r           toggle chronicle (full event history)"),
        Line::from(" ? / h       toggle this help"),
        Line::from(""),
        Line::from(Span::styled(
            " map legend ",
            Style::default()
                .fg(Color::Rgb(255, 220, 130))
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(" A ", Style::default().fg(C_MARKER_DEFAULT).add_modifier(Modifier::BOLD)),
            Span::raw("  settlement (name initial)"),
        ]),
        Line::from(vec![
            Span::styled(" A ", Style::default().fg(C_MARKER_MILITANT).add_modifier(Modifier::BOLD)),
            Span::raw("  militant (raids often)"),
        ]),
        Line::from(vec![
            Span::styled(" A ", Style::default().fg(C_MARKER_MERCANTILE).add_modifier(Modifier::BOLD)),
            Span::raw("  mercantile (haven of trade)"),
        ]),
        Line::from(vec![
            Span::styled(" A ", Style::default().fg(C_MARKER_DEPLETED).add_modifier(Modifier::BOLD)),
            Span::raw("  land around it depleted"),
        ]),
        Line::from(vec![
            Span::styled(" · ", Style::default().fg(C_MARKER_DEAD).add_modifier(Modifier::DIM)),
            Span::raw("  ruin (settlement fallen)"),
        ]),
        Line::from(vec![
            Span::styled(" · ", Style::default().fg(C_ROUTE_DECLARED)),
            Span::raw("  trade route"),
        ]),
        Line::from(vec![
            Span::styled(" • ", Style::default().fg(C_ROUTE_ALLIED)),
            Span::raw("  allied trade route"),
        ]),
        Line::from(vec![
            Span::styled(" ╳ ", Style::default().fg(C_FEUD)),
            Span::raw("  blood feud"),
        ]),
        Line::from(vec![
            Span::styled("   ", Style::default().bg(C_FLASH_BG)),
            Span::raw("  raid in progress (red flash on settlement)"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  press ? to close  ",
            Style::default().fg(Color::Rgb(160, 160, 160)),
        )),
    ];

    // Centered box sized to content.
    let content_w = 58u16.min(full.width.saturating_sub(4));
    let content_h = (lines.len() as u16 + 2).min(full.height.saturating_sub(2));
    let x = full.x + (full.width.saturating_sub(content_w)) / 2;
    let y = full.y + (full.height.saturating_sub(content_h)) / 2;
    let area = Rect::new(x, y, content_w, content_h);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" help ")
        .style(Style::default().bg(Color::Rgb(20, 22, 30)));
    let p = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}
