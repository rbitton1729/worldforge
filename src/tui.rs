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
                if handle_key(key, &mut ui) {
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
fn handle_key(key: KeyEvent, ui: &mut TuiState) -> bool {
    if key.kind == KeyEventKind::Release {
        return false;
    }
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
        KeyCode::Left => ui.cam_col -= 2,
        KeyCode::Right => ui.cam_col += 2,
        KeyCode::Up => {
            if ui.show_chronicle {
                ui.chronicle_scroll = ui.chronicle_scroll.saturating_sub(1);
            } else {
                ui.cam_row -= 2;
            }
        }
        KeyCode::Down => {
            if ui.show_chronicle {
                ui.chronicle_scroll = ui.chronicle_scroll.saturating_add(1);
            } else {
                ui.cam_row += 2;
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
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(1)])
        .split(outer);
    let main = vert[0];
    let help_bar = vert[1];

    let side_w = 36u16.min(outer.width / 3);
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(side_w)])
        .split(main);
    let map_area = body[0];
    let side = body[1];

    draw_help(f, help_bar, tick, sim_over, ui);
    draw_side(f, side, world, settlements, agents, tick, ui);
    if ui.show_chronicle {
        draw_chronicle(f, map_area, &ui.event_log, ui.chronicle_scroll);
    } else {
        draw_map(f, map_area, world, settlements, (ui.cam_col, ui.cam_row));
    }
}

fn speed_label(idx: usize) -> String {
    match SPEEDS[idx] {
        0 => "max".to_string(),
        n => format!("{}", n),
    }
}

fn draw_help(f: &mut Frame, area: Rect, tick: u64, sim_over: bool, ui: &TuiState) {
    let status = if sim_over {
        "ended"
    } else if ui.paused {
        "paused"
    } else {
        "running"
    };
    let view = if ui.show_chronicle { "map" } else { "log" };
    let text = format!(
        " [q]quit  [space]pause  [+/-]speed={}  [r]view→{}  [←↑↓→]pan    │ tick {}  year {}  {} ",
        speed_label(ui.speed_idx),
        view,
        tick,
        tick / TICKS_PER_YEAR + 1,
        status,
    );
    f.render_widget(
        Paragraph::new(text).style(
            Style::default()
                .bg(Color::Rgb(40, 40, 50))
                .fg(Color::Rgb(220, 220, 220)),
        ),
        area,
    );
}

fn draw_side(
    f: &mut Frame,
    area: Rect,
    world: &World,
    settlements: &Settlements,
    agents: &[Agent],
    tick: u64,
    ui: &TuiState,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Min(3)])
        .split(area);

    let pop = alive_count(agents);
    let settle_count = settlements.alive_count();
    let year = tick / TICKS_PER_YEAR + 1;
    let season = ["Spring", "Summer", "Autumn", "Winter"]
        [((tick % TICKS_PER_YEAR) / (TICKS_PER_YEAR / 4)) as usize];

    let stats_lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("Year       ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}  ({})", year, season), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("Tick       ", Style::default().fg(Color::DarkGray)),
            Span::raw(format!("{}", tick)),
        ]),
        Line::from(vec![
            Span::styled("Population ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}", pop), Style::default().fg(Color::Green)),
        ]),
        Line::from(vec![
            Span::styled("Settlements", Style::default().fg(Color::DarkGray)),
            Span::raw(format!(" {}", settle_count)),
        ]),
        Line::from(vec![
            Span::styled("Climate    ", Style::default().fg(Color::DarkGray)),
            Span::raw(format!("{:+.3}", world.climate_drift)),
        ]),
        Line::from(vec![
            Span::styled("Speed      ", Style::default().fg(Color::DarkGray)),
            Span::raw(format!("{} tps", speed_label(ui.speed_idx))),
        ]),
    ];
    f.render_widget(
        Paragraph::new(stats_lines)
            .block(Block::default().borders(Borders::ALL).title(" World ")),
        chunks[0],
    );

    // Recent events: most-recent N fitting the panel.
    let inner_h = chunks[1].height.saturating_sub(2) as usize;
    let visible: Vec<Line> = ui
        .event_log
        .iter()
        .rev()
        .take(inner_h.max(1) * 2) // over-provide; Wrap{trim} will drop excess
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|(t, text)| {
            Line::from(vec![
                Span::styled(
                    format!("t{:>5} ", t),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(text.clone()),
            ])
        })
        .collect();
    f.render_widget(
        Paragraph::new(visible)
            .block(Block::default().borders(Borders::ALL).title(" Events "))
            .wrap(Wrap { trim: true }),
        chunks[1],
    );
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

struct CellStyle {
    ch: char,
    fg: Color,
    bold: bool,
}

fn draw_map(
    f: &mut Frame,
    area: Rect,
    world: &World,
    settlements: &Settlements,
    cam: (i32, i32),
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" World  {}×{} ", world.width, world.height));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Half-block trick: one terminal row = two map rows stacked. Settlements
    // override the half-block with a bold marker character.
    let cols = inner.width as i32;
    let rows = inner.height as i32;

    let (cc, cr) = cam;
    let left = cc - cols / 2;
    let top = cr - rows;

    let mut settlement_at: HashMap<(i32, i32), (char, u32)> = HashMap::new();
    for s in &settlements.list {
        if !s.alive {
            continue;
        }
        let ch = s.name.chars().next().unwrap_or('#').to_ascii_uppercase();
        settlement_at.insert((s.col, s.row), (ch, s.population));
    }

    let mut lines: Vec<Line> = Vec::with_capacity(rows as usize);
    for term_row in 0..rows {
        let map_top = top + term_row * 2;
        let map_bot = map_top + 1;
        let mut spans: Vec<Span> = Vec::with_capacity(cols as usize);
        for term_col in 0..cols {
            let c = left + term_col;
            let top_cell = tile_style(world, &settlement_at, c, map_top);
            let bot_cell = tile_style(world, &settlement_at, c, map_bot);
            spans.push(combine_half_block(&top_cell, &bot_cell));
        }
        lines.push(Line::from(spans));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

fn tile_style(
    world: &World,
    settlement_at: &HashMap<(i32, i32), (char, u32)>,
    col: i32,
    row: i32,
) -> CellStyle {
    let Some(tile) = world.tile(col, row) else {
        return CellStyle {
            ch: ' ',
            fg: Color::Black,
            bold: false,
        };
    };
    let mut color = biome_color(tile.biome);
    if tile.river > 0 {
        color = river_color(tile.biome);
    }
    if let Some(&(ch, _pop)) = settlement_at.get(&(col, row)) {
        CellStyle {
            ch,
            fg: Color::Rgb(255, 240, 160),
            bold: true,
        }
    } else {
        CellStyle {
            ch: ' ',
            fg: color,
            bold: false,
        }
    }
}

/// Combine a top and bottom map row into one terminal cell. The upper half
/// takes the `top` biome color, the lower half the `bot` color. When either
/// cell is a settlement marker, render its character; priority: top, then bot.
fn combine_half_block(top: &CellStyle, bot: &CellStyle) -> Span<'static> {
    if top.ch != ' ' {
        // Settlement on the top map row — render its character using the
        // settlement's fg, with the bottom row's biome as the bg so the cell
        // stays embedded in the landscape.
        let style = Style::default()
            .fg(top.fg)
            .bg(bot.fg)
            .add_modifier(if top.bold { Modifier::BOLD } else { Modifier::empty() });
        return Span::styled(String::from(top.ch), style);
    }
    if bot.ch != ' ' {
        let style = Style::default()
            .fg(bot.fg)
            .bg(top.fg)
            .add_modifier(if bot.bold { Modifier::BOLD } else { Modifier::empty() });
        return Span::styled(String::from(bot.ch), style);
    }
    // Pure terrain: upper-half block paints top as fg, bottom as bg.
    Span::styled(
        "▀".to_string(),
        Style::default().fg(top.fg).bg(bot.fg),
    )
}

fn draw_chronicle(f: &mut Frame, area: Rect, log: &VecDeque<(u64, String)>, scroll: u16) {
    let block = Block::default().borders(Borders::ALL).title(" Chronicle ");
    let lines: Vec<Line> = log
        .iter()
        .map(|(t, text)| {
            Line::from(vec![
                Span::styled(
                    format!("t{:>6} ", t),
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
