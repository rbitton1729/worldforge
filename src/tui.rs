//! Real-time terminal UI for worldforge. Owns its own tick loop so pacing
//! can respond to user input; mirrors the sequence of phases in
//! `run_simulation` but interleaves rendering and keyboard events.

use crate::agent::{alive_count, seed_agents, step_agents, Agent};
use crate::chronicle::{Chronicle, Event, TICKS_PER_YEAR};
use crate::settlement::{update_settlements, Settlements};
use crate::world::{Biome, World};
use crate::{SimConfig, SimOutcome};

use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEvent, KeyEventKind},
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
    enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    Terminal::new(CrosstermBackend::new(out))
}

fn restore_terminal(term: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(
        term.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    term.show_cursor()?;
    Ok(())
}

struct TuiState {
    event_log: VecDeque<(u64, String)>,
    speed_idx: usize,
    paused: bool,
    show_chronicle: bool,
    zoom: Zoom,
    cam_col: i32,
    cam_row: i32,
    chronicle_scroll: u16,
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
        zoom: Zoom::Normal,
        cam_col: cfg.width as i32 / 2,
        cam_row: cfg.height as i32 / 2,
        chronicle_scroll: 0,
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
        KeyCode::Left => ui.cam_col -= pan,
        KeyCode::Right => ui.cam_col += pan,
        KeyCode::Up => {
            if ui.show_chronicle {
                ui.chronicle_scroll = ui.chronicle_scroll.saturating_sub(1);
            } else {
                ui.cam_row -= pan;
            }
        }
        KeyCode::Down => {
            if ui.show_chronicle {
                ui.chronicle_scroll = ui.chronicle_scroll.saturating_add(1);
            } else {
                ui.cam_row += pan;
            }
        }
        KeyCode::PageUp => {
            if ui.show_chronicle {
                ui.chronicle_scroll = ui.chronicle_scroll.saturating_sub(10);
            }
        }
        KeyCode::PageDown => {
            if ui.show_chronicle {
                ui.chronicle_scroll = ui.chronicle_scroll.saturating_add(10);
            }
        }
        _ => {}
    }
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
        draw_map(f, map_area, world, settlements, ui);
    }
    draw_events(f, events_area, &ui.event_log);
    draw_stats_bar(f, stats_area, world, settlements, agents, tick, sim_over, ui);
}

fn speed_label(idx: usize) -> String {
    match SPEEDS[idx] {
        0 => "max".to_string(),
        n => format!("{}", n),
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
    let year = tick / TICKS_PER_YEAR + 1;
    let season = ["Spring", "Summer", "Autumn", "Winter"]
        [((tick % TICKS_PER_YEAR) / (TICKS_PER_YEAR / 4)) as usize];
    let status = if sim_over {
        "ended"
    } else if ui.paused {
        "paused"
    } else {
        "running"
    };

    let bg = Color::Rgb(40, 40, 50);
    let fg = Color::Rgb(220, 220, 220);
    let dim = Color::Rgb(140, 140, 150);
    let accent = Color::Rgb(255, 220, 130);

    let sep = Span::styled(" │ ", Style::default().fg(dim).bg(bg));
    let stats_line = Line::from(vec![
        Span::styled(" Year ", Style::default().fg(dim).bg(bg)),
        Span::styled(
            format!("{} ({})", year, season),
            Style::default().fg(accent).bg(bg).add_modifier(Modifier::BOLD),
        ),
        sep.clone(),
        Span::styled("Pop ", Style::default().fg(dim).bg(bg)),
        Span::styled(format!("{}", pop), Style::default().fg(Color::LightGreen).bg(bg)),
        sep.clone(),
        Span::styled("Settlements ", Style::default().fg(dim).bg(bg)),
        Span::styled(format!("{}", settle_count), Style::default().fg(fg).bg(bg)),
        sep.clone(),
        Span::styled("Speed ", Style::default().fg(dim).bg(bg)),
        Span::styled(format!("{} tps", speed_label(ui.speed_idx)), Style::default().fg(fg).bg(bg)),
        sep.clone(),
        Span::styled("Zoom ", Style::default().fg(dim).bg(bg)),
        Span::styled(ui.zoom.label(), Style::default().fg(fg).bg(bg)),
        sep.clone(),
        Span::styled("Climate ", Style::default().fg(dim).bg(bg)),
        Span::styled(format!("{:+.2}", world.climate_drift), Style::default().fg(fg).bg(bg)),
        sep.clone(),
        Span::styled(status, Style::default().fg(accent).bg(bg)),
    ]);

    let keys_line = Line::from(vec![
        Span::styled(
            " [q]quit  [space]pause  [+/-]speed  [z]zoom  [r]chronicle  [←↑↓→]pan ",
            Style::default().fg(fg).bg(bg),
        ),
    ]);

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
        .map(|(t, text)| format_event_line(*t, text, inner_w))
        .collect();

    f.render_widget(Paragraph::new(lines), inner);
}

fn format_event_line(tick: u64, text: &str, max_width: usize) -> Line<'static> {
    let year = tick / TICKS_PER_YEAR + 1;
    let season = ["Sp", "Su", "Au", "Wi"]
        [((tick % TICKS_PER_YEAR) / (TICKS_PER_YEAR / 4)) as usize];
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
    Line::from(vec![
        Span::styled(prefix, Style::default().fg(Color::DarkGray)),
        Span::raw(body),
    ])
}

fn biome_color(b: Biome) -> Color {
    match b {
        Biome::Ocean => Color::Rgb(22, 60, 130),
        Biome::Coast => Color::Rgb(70, 180, 210),
        Biome::Plains => Color::Rgb(110, 180, 80),
        Biome::Forest => Color::Rgb(30, 100, 45),
        Biome::Hills => Color::Rgb(170, 150, 70),
        Biome::Mountains => Color::Rgb(225, 225, 230),
        Biome::Desert => Color::Rgb(215, 180, 105),
        Biome::Tundra => Color::Rgb(200, 220, 240),
    }
}

fn river_color(b: Biome) -> Color {
    if b == Biome::Ocean {
        biome_color(b)
    } else {
        Color::Rgb(80, 170, 220)
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

fn draw_map(
    f: &mut Frame,
    area: Rect,
    world: &World,
    settlements: &Settlements,
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

    match ui.zoom {
        Zoom::Normal => draw_map_halfblock(f, inner, world, settlements, ui, 1, false),
        Zoom::Out => {
            let cols = inner.width as u32;
            let halfrows = inner.height as u32 * 2;
            let sx = world.width.div_ceil(cols.max(1));
            let sy = world.height.div_ceil(halfrows.max(1));
            let scale = sx.max(sy).max(1);
            draw_map_halfblock(f, inner, world, settlements, ui, scale, true);
        }
        Zoom::In => draw_map_zoomed_in(f, inner, world, settlements, ui),
    }
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

    // Place settlement markers into terminal-cell slots (top or bottom half).
    // Pick the most-populated settlement if multiple fall into one half-cell.
    let mut marker_top: HashMap<(i32, i32), (char, u32)> = HashMap::new();
    let mut marker_bot: HashMap<(i32, i32), (char, u32)> = HashMap::new();
    for st in &settlements.list {
        if !st.alive {
            continue;
        }
        let dc = st.col - start_col;
        let dr = st.row - start_row;
        if dc < 0 || dr < 0 {
            continue;
        }
        let tc = dc / s;
        let strip = dr / s; // even=top, odd=bottom
        let tr = strip / 2;
        if tc >= cols || tr >= rows {
            continue;
        }
        let ch = st.name.chars().next().unwrap_or('#').to_ascii_uppercase();
        let slot = if strip % 2 == 0 {
            &mut marker_top
        } else {
            &mut marker_bot
        };
        slot.entry((tc, tr))
            .and_modify(|e| {
                if st.population > e.1 {
                    *e = (ch, st.population);
                }
            })
            .or_insert((ch, st.population));
    }

    let half_offset = s / 2;
    let mut lines: Vec<Line> = Vec::with_capacity(rows as usize);
    for term_row in 0..rows {
        let top_map_row = start_row + term_row * 2 * s + half_offset;
        let bot_map_row = top_map_row + s;
        let mut spans: Vec<Span> = Vec::with_capacity(cols as usize);
        for term_col in 0..cols {
            let map_col = start_col + term_col * s + half_offset;
            let top_color = tile_color(world, map_col, top_map_row);
            let bot_color = tile_color(world, map_col, bot_map_row);
            let top_ch = marker_top.get(&(term_col, term_row)).map(|e| e.0);
            let bot_ch = marker_bot.get(&(term_col, term_row)).map(|e| e.0);
            spans.push(combine_half_block(top_color, top_ch, bot_color, bot_ch));
        }
        lines.push(Line::from(spans));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

/// Zoomed-in renderer: each world tile occupies a 2×2 block of terminal cells.
/// Settlement marker is drawn in the top-left of its tile block.
fn draw_map_zoomed_in(
    f: &mut Frame,
    inner: Rect,
    world: &World,
    settlements: &Settlements,
    ui: &TuiState,
) {
    let tile_w: i32 = 2;
    let tile_h: i32 = 2;
    let tiles_cols = (inner.width as i32) / tile_w;
    let tiles_rows = (inner.height as i32) / tile_h;
    if tiles_cols <= 0 || tiles_rows <= 0 {
        return;
    }

    let left = ui.cam_col - tiles_cols / 2;
    let top = ui.cam_row - tiles_rows / 2;

    let mut settlement_at: HashMap<(i32, i32), char> = HashMap::new();
    for st in &settlements.list {
        if !st.alive {
            continue;
        }
        let ch = st.name.chars().next().unwrap_or('#').to_ascii_uppercase();
        settlement_at.insert((st.col, st.row), ch);
    }

    let total_rows = (tiles_rows * tile_h) as usize;
    let total_cols = (tiles_cols * tile_w) as usize;
    let mut lines: Vec<Line> = Vec::with_capacity(total_rows);
    for term_row in 0..(tiles_rows * tile_h) {
        let tile_row = top + term_row / tile_h;
        let sub_row = term_row % tile_h;
        let mut spans: Vec<Span> = Vec::with_capacity(total_cols);
        for tile_col_off in 0..tiles_cols {
            let tile_col = left + tile_col_off;
            let color = tile_color(world, tile_col, tile_row);
            let marker = settlement_at.get(&(tile_col, tile_row)).copied();
            for sub_col in 0..tile_w {
                let is_marker_cell = marker.is_some() && sub_row == 0 && sub_col == 0;
                if is_marker_cell {
                    let ch = marker.unwrap();
                    spans.push(Span::styled(
                        String::from(ch),
                        Style::default()
                            .fg(Color::Rgb(255, 240, 160))
                            .bg(color)
                            .add_modifier(Modifier::BOLD),
                    ));
                } else {
                    spans.push(Span::styled(" ".to_string(), Style::default().bg(color)));
                }
            }
        }
        lines.push(Line::from(spans));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

/// Combine a top and bottom map sample into one terminal cell using the
/// upper-half-block character. When a settlement marker is present in either
/// half, render the marker char instead, with the other half's biome as bg.
fn combine_half_block(
    top_color: Color,
    top_ch: Option<char>,
    bot_color: Color,
    bot_ch: Option<char>,
) -> Span<'static> {
    if let Some(ch) = top_ch {
        return Span::styled(
            String::from(ch),
            Style::default()
                .fg(Color::Rgb(255, 240, 160))
                .bg(bot_color)
                .add_modifier(Modifier::BOLD),
        );
    }
    if let Some(ch) = bot_ch {
        return Span::styled(
            String::from(ch),
            Style::default()
                .fg(Color::Rgb(255, 240, 160))
                .bg(top_color)
                .add_modifier(Modifier::BOLD),
        );
    }
    Span::styled(
        "▀".to_string(),
        Style::default().fg(top_color).bg(bot_color),
    )
}

fn draw_chronicle(f: &mut Frame, area: Rect, log: &VecDeque<(u64, String)>, scroll: u16) {
    let block = Block::default().borders(Borders::ALL).title(" Chronicle ");
    let lines: Vec<Line> = log
        .iter()
        .map(|(t, text)| {
            let year = t / TICKS_PER_YEAR + 1;
            let season = ["Sp", "Su", "Au", "Wi"]
                [((t % TICKS_PER_YEAR) / (TICKS_PER_YEAR / 4)) as usize];
            Line::from(vec![
                Span::styled(
                    format!("Y{:<5} {} ", year, season),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(text.clone()),
            ])
        })
        .collect();
    let p = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: true })
        .scroll((scroll, 0));
    f.render_widget(p, area);
}
