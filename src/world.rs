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
            Biome::Plains => 0.05,
            Biome::Forest => 0.08,
            Biome::Coast => 0.06,
            Biome::Hills => 0.03,
            Biome::Tundra => 0.01,
            Biome::Desert => 0.005,
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

    pub fn regen_food(&mut self) {
        for tile in &mut self.tiles {
            let regen = tile.biome.food_regen();
            let cap = tile.biome.food_cap();
            if regen > 0.0 && tile.food < cap {
                tile.food = (tile.food + regen).min(cap);
            }
        }
    }
}

fn offset_to_cube(col: i32, row: i32) -> (i32, i32) {
    // odd-r offset to cube (x, z); y = -x - z
    let x = col - ((row - (row & 1)) / 2);
    let z = row;
    (x, z)
}
