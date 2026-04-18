use std::fs::File;
use std::io::{self, BufWriter, Write};

pub const TICKS_PER_YEAR: u64 = 100;
pub const SEASONS: [&str; 4] = ["Spring", "Summer", "Autumn", "Winter"];

const RESET: &str = "\x1b[0m";
const BOLD_WHITE: &str = "\x1b[1;37m";
const BOLD_YELLOW: &str = "\x1b[1;33m";
const BOLD_GREEN: &str = "\x1b[1;32m";
const BOLD_CYAN: &str = "\x1b[1;36m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const CYAN: &str = "\x1b[36m";
const MAGENTA: &str = "\x1b[35m";
const YELLOW: &str = "\x1b[33m";
const WHITE: &str = "\x1b[37m";
const DIM_RED: &str = "\x1b[2;31m";

/// Choose an ANSI color for a chronicle line based on its content. Returns
/// (prefix, suffix) to wrap the line; empty strings mean "don't colorize".
fn colors_for(line: &str) -> (&'static str, &'static str) {
    let trimmed = line.trim_start();
    if trimmed.starts_with("--- Year") {
        return (BOLD_WHITE, RESET);
    }
    if trimmed.starts_with("***") {
        return (BOLD_YELLOW, RESET);
    }
    if trimmed.starts_with("The chronicle closes") || trimmed.starts_with("Silence falls") {
        return (BOLD_WHITE, RESET);
    }
    if trimmed.contains("souls draw their first breath") {
        return (BOLD_GREEN, RESET);
    }
    if trimmed.contains("becomes known") {
        return (BOLD_CYAN, RESET);
    }
    if trimmed.contains("is abandoned") {
        return (DIM_RED, RESET);
    }
    if trimmed.contains("perishes") || trimmed.contains("dies of old age") {
        return (RED, RESET);
    }
    if trimmed.contains("descend upon") || trimmed.contains("sacks") {
        return (RED, RESET);
    }
    if trimmed.contains("repel") {
        return (MAGENTA, RESET);
    }
    if trimmed.contains("merchant arrives") || trimmed.contains("bearing grain") {
        return (CYAN, RESET);
    }
    if trimmed.contains("granary") && trimmed.contains("overflows") {
        return (GREEN, RESET);
    }
    if trimmed.contains("A band of") || trimmed.contains("settlers gathers") {
        return (GREEN, RESET);
    }
    if trimmed.contains("depart the starving halls") {
        return (YELLOW, RESET);
    }
    if trimmed.contains("holds")
        || trimmed.contains("dwindles")
        || trimmed.contains("thrives with")
        || trimmed.contains("endures with")
        || trimmed.contains("the living number")
    {
        return (WHITE, RESET);
    }
    ("", "")
}

/// A single narrated event emitted by the simulation.
#[derive(Debug, Clone)]
pub struct Event {
    pub tick: u64,
    pub text: String,
}

impl Event {
    pub fn new(tick: u64, text: String) -> Self {
        Self { tick, text }
    }
}

/// Collects events during a tick, then emits them grouped under a season header
/// the first time a new season appears. Writes to stdout or a file.
pub struct Chronicle {
    writer: Box<dyn Write>,
    pending: Vec<Event>,
    last_header: Option<(u64, u64)>, // (year, season_index)
    header_stats: Option<(usize, usize)>, // (alive souls, alive settlements)
    color: bool,
    last_foraging_milestone_year: Option<u64>,
}

impl Chronicle {
    pub fn to_stdout() -> Self {
        let color = std::env::var_os("NO_COLOR").is_none();
        Self {
            writer: Box::new(BufWriter::new(io::stdout())),
            pending: Vec::new(),
            last_header: None,
            header_stats: None,
            color,
            last_foraging_milestone_year: None,
        }
    }

    pub fn to_file(path: &str) -> io::Result<Self> {
        let file = File::create(path)?;
        Ok(Self {
            writer: Box::new(BufWriter::new(file)),
            pending: Vec::new(),
            last_header: None,
            header_stats: None,
            color: false,
            last_foraging_milestone_year: None,
        })
    }

    /// Discard all output. Used by tests that don't need the chronicle.
    pub fn sink() -> Self {
        Self {
            writer: Box::new(io::sink()),
            pending: Vec::new(),
            last_header: None,
            header_stats: None,
            color: false,
            last_foraging_milestone_year: None,
        }
    }

    /// Rate-limit the "grows masterful at finding food" line to at most once
    /// per simulated year across the whole world. Agents churn fast; without
    /// this gate the line dominates the chronicle even after per-agent dedup.
    /// Returns true (and records the year) when the caller may emit.
    pub fn try_foraging_milestone_year(&mut self, tick: u64) -> bool {
        let year = tick / TICKS_PER_YEAR;
        if self.last_foraging_milestone_year == Some(year) {
            return false;
        }
        self.last_foraging_milestone_year = Some(year);
        true
    }

    /// Enable or disable ANSI coloring. Used for --no-color overrides.
    pub fn set_color(&mut self, color: bool) {
        self.color = color;
    }

    fn write_colored(&mut self, line: &str) -> io::Result<()> {
        if self.color {
            let (pre, post) = colors_for(line);
            if !pre.is_empty() {
                return writeln!(self.writer, "{}{}{}", pre, line, post);
            }
        }
        writeln!(self.writer, "{}", line)
    }

    pub fn record(&mut self, event: Event) {
        self.pending.push(event);
    }

    /// Update the souls/settlements counts woven into the next season header.
    pub fn set_header_stats(&mut self, alive_souls: usize, alive_settlements: usize) {
        self.header_stats = Some((alive_souls, alive_settlements));
    }

    /// Emit a top-level line (no season header). Useful for prologue and epilogue.
    pub fn proclaim(&mut self, text: &str) -> io::Result<()> {
        for line in text.split('\n') {
            self.write_colored(line)?;
        }
        self.writer.flush()
    }

    /// Flush all pending events for this tick. Season headers are written only
    /// when the current (year, season) differs from the last emitted one.
    pub fn flush_tick(&mut self, tick: u64) -> io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }

        let year = tick / TICKS_PER_YEAR + 1;
        let season_idx = (tick % TICKS_PER_YEAR) / (TICKS_PER_YEAR / 4);
        let header_key = (year, season_idx);

        if self.last_header != Some(header_key) {
            let header = match self.header_stats {
                Some((souls, settlements)) => format!(
                    "--- Year {}, {} — {} souls across {} settlements ---",
                    year, SEASONS[season_idx as usize], souls, settlements
                ),
                None => format!(
                    "--- Year {}, {} ---",
                    year, SEASONS[season_idx as usize]
                ),
            };
            writeln!(self.writer)?;
            self.write_colored(&header)?;
            self.last_header = Some(header_key);
        }

        let events: Vec<Event> = self.pending.drain(..).collect();
        for ev in events {
            self.write_colored(&ev.text)?;
        }
        self.writer.flush()
    }
}

pub fn describe_season(tick: u64) -> String {
    let year = tick / TICKS_PER_YEAR + 1;
    let season_idx = (tick % TICKS_PER_YEAR) / (TICKS_PER_YEAR / 4);
    format!("Year {}, {}", year, SEASONS[season_idx as usize])
}
