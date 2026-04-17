use std::fs::File;
use std::io::{self, BufWriter, Write};

pub const TICKS_PER_YEAR: u64 = 100;
pub const SEASONS: [&str; 4] = ["Spring", "Summer", "Autumn", "Winter"];

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
}

impl Chronicle {
    pub fn to_stdout() -> Self {
        Self {
            writer: Box::new(BufWriter::new(io::stdout())),
            pending: Vec::new(),
            last_header: None,
        }
    }

    pub fn to_file(path: &str) -> io::Result<Self> {
        let file = File::create(path)?;
        Ok(Self {
            writer: Box::new(BufWriter::new(file)),
            pending: Vec::new(),
            last_header: None,
        })
    }

    pub fn record(&mut self, event: Event) {
        self.pending.push(event);
    }

    /// Emit a top-level line (no season header). Useful for prologue and epilogue.
    pub fn proclaim(&mut self, text: &str) -> io::Result<()> {
        writeln!(self.writer, "{}", text)?;
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
            writeln!(
                self.writer,
                "\n--- Year {}, {} ---",
                year, SEASONS[season_idx as usize]
            )?;
            self.last_header = Some(header_key);
        }

        for ev in self.pending.drain(..) {
            writeln!(self.writer, "{}", ev.text)?;
        }
        self.writer.flush()
    }
}

pub fn describe_season(tick: u64) -> String {
    let year = tick / TICKS_PER_YEAR + 1;
    let season_idx = (tick % TICKS_PER_YEAR) / (TICKS_PER_YEAR / 4);
    format!("Year {}, {}", year, SEASONS[season_idx as usize])
}
