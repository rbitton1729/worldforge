use crate::region::{self, Region};
use crate::worldgen;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Biome {
    Ocean,
    Coast,
    Plains,
    Forest,
    Hills,
    Mountains,
    Desert,
    Tundra,
}

impl Biome {
    pub fn is_passable(self) -> bool {
        !matches!(self, Biome::Ocean)
    }

    /// Base food regenerated per tick per tile (scaled small).
    pub fn food_regen(self) -> f32 {
        match self {
            Biome::Plains => 0.020,
            Biome::Forest => 0.030,
            Biome::Coast => 0.022,
            Biome::Hills => 0.010,
            Biome::Tundra => 0.003,
            Biome::Desert => 0.001,
            Biome::Mountains => 0.0,
            Biome::Ocean => 0.0,
        }
    }

    /// Max food the tile can hold.
    pub fn food_cap(self) -> f32 {
        match self {
            Biome::Plains => 9.0,
            Biome::Forest => 15.0,
            Biome::Coast => 9.0,
            Biome::Hills => 4.0,
            Biome::Tundra => 1.5,
            Biome::Desert => 0.5,
            Biome::Mountains => 0.0,
            Biome::Ocean => 0.0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Biome::Ocean => "ocean",
            Biome::Coast => "coast",
            Biome::Plains => "plains",
            Biome::Forest => "forest",
            Biome::Hills => "hills",
            Biome::Mountains => "mountains",
            Biome::Desert => "desert",
            Biome::Tundra => "tundra",
        }
    }

    /// The natural upper bound on a tile's fertility. Deserts and tundra are
    /// already "depleted" by nature and can't recover past a low baseline.
    pub fn natural_fertility(self) -> f32 {
        match self {
            Biome::Plains => 1.0,
            Biome::Forest => 1.0,
            Biome::Coast => 0.9,
            Biome::Hills => 0.8,
            Biome::Desert => 0.3,
            Biome::Tundra => 0.3,
            Biome::Mountains => 0.0,
            Biome::Ocean => 0.0,
        }
    }
}

/// Fertility lost per full (2.0-unit) bite of foraged food. Partial bites
/// scale proportionally, so the cost tracks how much the agent actually ate.
pub const FERTILITY_PER_BITE: f32 = 0.02;
/// Fertility recovered per tick, before the season multiplier is applied.
pub const FERTILITY_RECOVERY: f32 = 0.001;
/// Floor applied to fertility's multiplier on food cap — a fully depleted
/// tile still holds a fraction of its base capacity.
pub const FERTILITY_CAP_FLOOR: f32 = 0.2;

/// Absolute bound on climate drift — keeps the world this side of an ice age.
pub const CLIMATE_DRIFT_MIN: f32 = -0.15;
pub const CLIMATE_DRIFT_MAX: f32 = 0.15;
/// Drift threshold at which elders start remarking on the warmth.
pub const CLIMATE_WARM_THRESHOLD: f32 = 0.08;
pub const CLIMATE_COLD_THRESHOLD: f32 = -0.08;
/// Baseline drift magnitude per year (±).
const CLIMATE_DRIFT_MAGNITUDE: f32 = 0.0005;
/// Drift has to relax back inside this band before the warm/cold event can refire.
const CLIMATE_NOTIFY_RESET: f32 = 0.04;

#[derive(Debug, Clone)]
pub struct Tile {
    pub biome: Biome,
    pub elevation: f32,
    pub temperature: f32,
    pub moisture: f32,
    pub food: f32,
    pub river: u8,
    /// Land health on [0.0, 1.0]. Foraging depletes it; unused land recovers
    /// up to the biome's natural_fertility() cap.
    pub fertility: f32,
}

pub struct World {
    pub width: u32,
    pub height: u32,
    pub tiles: Vec<Tile>,
    pub regions: Vec<Region>,
    /// Per-tile lookup into `regions`. None for tiles outside any named region.
    region_of: Vec<Option<u16>>,
    /// Long-timescale temperature drift in `[CLIMATE_DRIFT_MIN, CLIMATE_DRIFT_MAX]`.
    pub climate_drift: f32,
    climate_direction: f32,
    climate_next_flip_year: u64,
    climate_warm_notified: bool,
    climate_cold_notified: bool,
    climate_rng: ChaCha8Rng,
}

impl World {
    pub fn generate(width: u32, height: u32, seed: u64) -> Self {
        let tiles = worldgen::generate_tiles(width, height, seed);
        let (regions, region_of) = region::detect_regions(&tiles, width, height, seed);
        let mut climate_rng = ChaCha8Rng::seed_from_u64(seed ^ 0xC11A_7ED_D11F7);
        let dir_sign: f32 = if climate_rng.gen_bool(0.5) { 1.0 } else { -1.0 };
        let dir_scale: f32 = climate_rng.gen_range(0.6f32..=1.4f32);
        let climate_direction = dir_sign * CLIMATE_DRIFT_MAGNITUDE * dir_scale;
        let climate_next_flip_year = climate_rng.gen_range(50..=100);
        Self {
            width,
            height,
            tiles,
            regions,
            region_of,
            climate_drift: 0.0,
            climate_direction,
            climate_next_flip_year,
            climate_warm_notified: false,
            climate_cold_notified: false,
            climate_rng,
        }
    }

    /// Return the region containing (col, row), if any.
    pub fn region_at(&self, col: i32, row: i32) -> Option<&Region> {
        let i = self.idx(col, row)?;
        self.region_of[i].map(|ri| &self.regions[ri as usize])
    }

    /// Names of the N largest regions by tile count, for the prologue.
    pub fn major_region_names(&self, n: usize) -> Vec<String> {
        let mut by_size: Vec<&Region> = self.regions.iter().collect();
        by_size.sort_by(|a, b| b.tile_count.cmp(&a.tile_count));
        by_size.iter().take(n).map(|r| r.name.clone()).collect()
    }

    #[inline]
    pub fn idx(&self, col: i32, row: i32) -> Option<usize> {
        if col < 0 || row < 0 || (col as u32) >= self.width || (row as u32) >= self.height {
            return None;
        }
        Some((row as usize) * self.width as usize + col as usize)
    }

    pub fn tile(&self, col: i32, row: i32) -> Option<&Tile> {
        self.idx(col, row).map(|i| &self.tiles[i])
    }

    pub fn tile_mut(&mut self, col: i32, row: i32) -> Option<&mut Tile> {
        let i = self.idx(col, row)?;
        Some(&mut self.tiles[i])
    }

    /// Odd-r offset hex neighbors.
    pub fn neighbors(&self, col: i32, row: i32) -> [(i32, i32); 6] {
        let odd = row & 1 == 1;
        if odd {
            [
                (col, row - 1),
                (col + 1, row - 1),
                (col - 1, row),
                (col + 1, row),
                (col, row + 1),
                (col + 1, row + 1),
            ]
        } else {
            [
                (col - 1, row - 1),
                (col, row - 1),
                (col - 1, row),
                (col + 1, row),
                (col - 1, row + 1),
                (col, row + 1),
            ]
        }
    }

    /// Hex distance in offset (odd-r) coordinates.
    pub fn hex_distance(&self, a: (i32, i32), b: (i32, i32)) -> i32 {
        let (ax, az) = offset_to_cube(a.0, a.1);
        let (bx, bz) = offset_to_cube(b.0, b.1);
        let ay = -ax - az;
        let by = -bx - bz;
        ((ax - bx).abs() + (ay - by).abs() + (az - bz).abs()) / 2
    }

    /// Is (col, row) inside the map and land?
    pub fn is_land(&self, col: i32, row: i32) -> bool {
        self.tile(col, row).is_some_and(|t| t.biome.is_passable())
    }

    /// Check if a tile has a river on it or is adjacent to one.
    pub fn is_near_river(&self, col: i32, row: i32) -> bool {
        if let Some(i) = self.idx(col, row)
            && self.tiles[i].river > 0 {
                return true;
            }
        for (nc, nr) in self.neighbors(col, row) {
            if let Some(ni) = self.idx(nc, nr)
                && self.tiles[ni].river > 0 {
                    return true;
                }
        }
        false
    }

    pub fn regen_food(&mut self, tick: u64) {
        let factor = season_regen_factor(tick);
        let climate_factor = (1.0 + self.climate_drift * 0.5).max(0.0);
        // Precompute river-adjacency so the subsequent mut borrow of tiles is clean.
        let mut bonus = vec![false; self.tiles.len()];
        for row in 0..self.height as i32 {
            for col in 0..self.width as i32 {
                if self.is_near_river(col, row) {
                    bonus[self.idx(col, row).unwrap()] = true;
                }
            }
        }
        for (i, tile) in self.tiles.iter_mut().enumerate() {
            let (regen_mul, cap_mul) = if bonus[i] { (1.5, 1.5) } else { (1.0, 1.0) };
            let fert = tile.fertility;
            let regen = tile.biome.food_regen() * factor * climate_factor * regen_mul * fert;
            let cap = tile.biome.food_cap() * cap_mul * fert.max(FERTILITY_CAP_FLOOR);
            if regen > 0.0 && tile.food < cap {
                tile.food = (tile.food + regen).min(cap);
            }
            // Fertility slowly heals even on foraged tiles, with a bonus when
            // the plot is well-stocked (so ungrazed land recovers fastest).
            let natural = tile.biome.natural_fertility();
            if fert < natural && cap > 0.0 {
                let heal_mul = if tile.food >= cap * 0.7 { 1.0 } else { 0.3 };
                tile.fertility = (fert + FERTILITY_RECOVERY * factor * heal_mul).min(natural);
            }
        }
    }

    /// Advance the world's long-term climate drift. Intended to be called once
    /// per year (on tick boundaries `% TICKS_PER_YEAR == 0`); calls on other
    /// ticks are no-ops. Returns a chronicle line if a drift threshold was
    /// freshly crossed this year.
    pub fn tick_climate(&mut self, tick: u64) -> Option<&'static str> {
        let ticks_per_year = crate::chronicle::TICKS_PER_YEAR;
        if tick == 0 || !tick.is_multiple_of(ticks_per_year) {
            return None;
        }
        let year = tick / ticks_per_year;
        self.climate_drift = (self.climate_drift + self.climate_direction)
            .clamp(CLIMATE_DRIFT_MIN, CLIMATE_DRIFT_MAX);

        if year >= self.climate_next_flip_year {
            let dir_sign: f32 = if self.climate_rng.gen_bool(0.5) {
                1.0
            } else {
                -1.0
            };
            let dir_scale: f32 = self.climate_rng.gen_range(0.6f32..=1.4f32);
            self.climate_direction = dir_sign * CLIMATE_DRIFT_MAGNITUDE * dir_scale;
            self.climate_next_flip_year = year + self.climate_rng.gen_range(50..=100);
        }

        let mut event: Option<&'static str> = None;
        if self.climate_drift > CLIMATE_WARM_THRESHOLD {
            if !self.climate_warm_notified {
                self.climate_warm_notified = true;
                event = Some("The seasons grow warmer than any elder remembers.");
            }
        } else if self.climate_drift < CLIMATE_NOTIFY_RESET {
            self.climate_warm_notified = false;
        }
        if self.climate_drift < CLIMATE_COLD_THRESHOLD {
            if !self.climate_cold_notified {
                self.climate_cold_notified = true;
                event = Some("An unseasonable cold settles over the land.");
            }
        } else if self.climate_drift > -CLIMATE_NOTIFY_RESET {
            self.climate_cold_notified = false;
        }
        event
    }

    /// Count distinct rivers (connected components of river tiles).
    pub fn river_count(&self) -> u32 {
        let mut visited = vec![false; self.tiles.len()];
        let mut count = 0u32;
        for row in 0..self.height as i32 {
            for col in 0..self.width as i32 {
                let i = match self.idx(col, row) {
                    Some(i) => i,
                    None => continue,
                };
                if visited[i] || self.tiles[i].river == 0 {
                    continue;
                }
                count += 1;
                let mut stack = vec![(col, row)];
                while let Some((c, r)) = stack.pop() {
                    let idx = match self.idx(c, r) {
                        Some(i) => i,
                        None => continue,
                    };
                    if visited[idx] || self.tiles[idx].river == 0 {
                        continue;
                    }
                    visited[idx] = true;
                    for (nc, nr) in self.neighbors(c, r) {
                        stack.push((nc, nr));
                    }
                }
            }
        }
        count
    }
}

/// Season multiplier for food regeneration. Spring/Summer lush, Autumn lean, Winter brutal.
pub fn season_regen_factor(tick: u64) -> f32 {
    let ticks_per_year = crate::chronicle::TICKS_PER_YEAR;
    let season = (tick % ticks_per_year) / (ticks_per_year / 4);
    match season {
        0 => 1.3, // Spring
        1 => 1.6, // Summer
        2 => 0.5, // Autumn
        _ => 0.2, // Winter
    }
}

fn offset_to_cube(col: i32, row: i32) -> (i32, i32) {
    // odd-r offset to cube (x, z); y = -x - z
    let x = col - ((row - (row & 1)) / 2);
    let z = row;
    (x, z)
}
