use crate::worldgen;

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
        !matches!(self, Biome::Ocean | Biome::Mountains)
    }

    /// Base food regenerated per tick per tile (scaled small).
    pub fn food_regen(self) -> f32 {
        match self {
            Biome::Plains => 0.012,
            Biome::Forest => 0.020,
            Biome::Coast => 0.015,
            Biome::Hills => 0.007,
            Biome::Tundra => 0.002,
            Biome::Desert => 0.001,
            Biome::Mountains => 0.0,
            Biome::Ocean => 0.0,
        }
    }

    /// Max food the tile can hold.
    pub fn food_cap(self) -> f32 {
        match self {
            Biome::Plains => 6.0,
            Biome::Forest => 10.0,
            Biome::Coast => 7.0,
            Biome::Hills => 3.0,
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
}

#[derive(Debug, Clone)]
pub struct Tile {
    pub biome: Biome,
    pub elevation: f32,
    pub temperature: f32,
    pub moisture: f32,
    pub food: f32,
    pub river: u8,
}

pub struct World {
    pub width: u32,
    pub height: u32,
    pub tiles: Vec<Tile>,
}

impl World {
    pub fn generate(width: u32, height: u32, seed: u64) -> Self {
        let tiles = worldgen::generate_tiles(width, height, seed);
        Self {
            width,
            height,
            tiles,
        }
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
        self.tile(col, row).map_or(false, |t| t.biome.is_passable())
    }

    /// Check if a tile has a river on it or is adjacent to one.
    pub fn is_near_river(&self, col: i32, row: i32) -> bool {
        if let Some(i) = self.idx(col, row) {
            if self.tiles[i].river > 0 {
                return true;
            }
        }
        for (nc, nr) in self.neighbors(col, row) {
            if let Some(ni) = self.idx(nc, nr) {
                if self.tiles[ni].river > 0 {
                    return true;
                }
            }
        }
        false
    }

    pub fn regen_food(&mut self, tick: u64) {
        let factor = season_regen_factor(tick);
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
            let regen = tile.biome.food_regen() * factor * regen_mul;
            let cap = tile.biome.food_cap() * cap_mul;
            if regen > 0.0 && tile.food < cap {
                tile.food = (tile.food + regen).min(cap);
            }
        }
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
        _ => 0.05, // Winter
    }
}

fn offset_to_cube(col: i32, row: i32) -> (i32, i32) {
    // odd-r offset to cube (x, z); y = -x - z
    let x = col - ((row - (row & 1)) / 2);
    let z = row;
    (x, z)
}
