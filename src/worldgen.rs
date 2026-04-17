use crate::world::{Biome, Tile};
use noise::{NoiseFn, Perlin};

/// Generate a flat vector of tiles for a `width × height` hex map.
///
/// Three noise layers — elevation, temperature, moisture — are blended to
/// pick a biome per tile. Latitude biases temperature so the poles are
/// colder than the equator, which gives tundra at top/bottom and desert
/// bands around the middle.
pub fn generate_tiles(width: u32, height: u32, seed: u64) -> Vec<Tile> {
    let elev_noise = Perlin::new(seed as u32);
    let temp_noise = Perlin::new(seed.wrapping_add(1) as u32);
    let moist_noise = Perlin::new(seed.wrapping_add(2) as u32);
    let detail_noise = Perlin::new(seed.wrapping_add(3) as u32);

    let mut tiles = Vec::with_capacity((width * height) as usize);

    for row in 0..height {
        for col in 0..width {
            let (fx, fy) = tile_to_point(col, row);

            // Layered fractal noise for elevation.
            let e = fbm(&elev_noise, fx * 0.05, fy * 0.05, 4, 0.5, 2.0);
            // Pull elevation down toward the map edges so oceans frame the world.
            let edge = edge_falloff(col as f32, row as f32, width as f32, height as f32);
            let elevation = normalize(e) * edge;

            // Temperature: mostly from latitude, slight noise wiggle.
            let lat = (row as f32 / (height as f32 - 1.0) - 0.5).abs() * 2.0; // 0 at equator, 1 at poles
            let t_noise = normalize(temp_noise.get([fx as f64 * 0.04, fy as f64 * 0.04]) as f32);
            let mut temperature = 1.0 - lat + (t_noise - 0.5) * 0.3;
            // High elevation is cold.
            temperature -= (elevation - 0.5).max(0.0) * 0.6;
            temperature = temperature.clamp(0.0, 1.0);

            let m = fbm(&moist_noise, fx * 0.06, fy * 0.06, 3, 0.5, 2.0);
            let moisture = normalize(m);

            let _d = detail_noise.get([fx as f64 * 0.2, fy as f64 * 0.2]) as f32;

            let biome = pick_biome(elevation, temperature, moisture);

            // Tiles start half-stocked so early agents have something to forage.
            let food = biome.food_cap() * 0.5;

            tiles.push(Tile {
                biome,
                elevation,
                temperature,
                moisture,
                food,
            });
        }
    }

    tiles
}

/// Convert odd-r offset coordinates to a planar sample point. Odd rows are
/// shifted half a tile east so the noise reads as a true hex field instead
/// of aligned rows.
fn tile_to_point(col: u32, row: u32) -> (f32, f32) {
    let x = col as f32 + if row & 1 == 1 { 0.5 } else { 0.0 };
    let y = row as f32 * 0.866; // sqrt(3)/2, flat-top-ish spacing
    (x, y)
}

fn fbm(noise: &Perlin, x: f32, y: f32, octaves: u32, persistence: f32, lacunarity: f32) -> f32 {
    let mut total = 0.0;
    let mut amplitude = 1.0;
    let mut frequency = 1.0;
    let mut max_value = 0.0;
    for _ in 0..octaves {
        total += noise.get([(x * frequency) as f64, (y * frequency) as f64]) as f32 * amplitude;
        max_value += amplitude;
        amplitude *= persistence;
        frequency *= lacunarity;
    }
    total / max_value
}

fn normalize(v: f32) -> f32 {
    (v * 0.5 + 0.5).clamp(0.0, 1.0)
}

/// Radial-ish falloff that pulls values down toward the rectangle edges.
fn edge_falloff(x: f32, y: f32, w: f32, h: f32) -> f32 {
    let nx = (x / (w - 1.0)) * 2.0 - 1.0;
    let ny = (y / (h - 1.0)) * 2.0 - 1.0;
    let d = (nx * nx + ny * ny).sqrt();
    // 1.0 at center, decays toward 0 near edges.
    (1.0 - d * 0.55).clamp(0.0, 1.0)
}

fn pick_biome(elevation: f32, temperature: f32, moisture: f32) -> Biome {
    if elevation < 0.30 {
        return Biome::Ocean;
    }
    if elevation < 0.34 {
        return Biome::Coast;
    }
    if elevation > 0.55 {
        return Biome::Mountains;
    }
    if temperature < 0.22 {
        return Biome::Tundra;
    }
    if elevation > 0.49 {
        return Biome::Hills;
    }
    if temperature > 0.7 && moisture < 0.35 {
        return Biome::Desert;
    }
    if moisture > 0.55 {
        return Biome::Forest;
    }
    Biome::Plains
}

#[cfg(test)]
mod probe_tests {
    use super::*;
    #[test]
    fn probe_elev() {
        let tiles = generate_tiles(80, 40, 42);
        let mut elevs: Vec<f32> = tiles.iter().map(|t| t.elevation).collect();
        elevs.sort_by(|a,b| a.partial_cmp(b).unwrap());
        let n = elevs.len();
        for p in [0.50, 0.70, 0.80, 0.85, 0.90, 0.95, 0.98, 0.99, 1.00] {
            let i = ((n as f32 * p) as usize).min(n-1);
            eprintln!("p{:.2} = {:.3}", p, elevs[i]);
        }
        eprintln!("max = {:.3}", elevs[n-1]);
    }
}
