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

            let biome = pick_biome(elevation, temperature, moisture);

            let fertility = biome.natural_fertility();
            // Tiles start half-stocked so early agents have something to forage.
            let food = biome.food_cap() * fertility * 0.5;

            tiles.push(Tile {
                biome,
                elevation,
                temperature,
                moisture,
                food,
                river: 0,
                fertility,
            });
        }
    }

    generate_rivers(&mut tiles, width, height, seed);
    tiles
}

/// Odd-r offset hex neighbors for a standalone tile grid.
fn hex_neighbors(col: i32, row: i32) -> [(i32, i32); 6] {
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

fn tile_idx(col: i32, row: i32, width: u32, height: u32) -> Option<usize> {
    if col < 0 || row < 0 || col as u32 >= width || row as u32 >= height {
        return None;
    }
    Some(row as usize * width as usize + col as usize)
}

/// Generate rivers flowing from high ground to the coast.
/// Uses seeded RNG to pick sources and determine flow paths.
fn generate_rivers(tiles: &mut [Tile], width: u32, height: u32, seed: u64) {
    use rand::SeedableRng;
    use rand::Rng;
    use rand_chacha::ChaCha8Rng;

    let mut rng = ChaCha8Rng::seed_from_u64(seed.wrapping_add(777));

    // Collect potential sources: hill and mountain tiles sorted by elevation (highest first).
    let mut sources: Vec<(i32, i32, f32)> = Vec::new();
    for row in 0..height as i32 {
        for col in 0..width as i32 {
            let i = tile_idx(col, row, width, height).unwrap();
            let e = tiles[i].elevation;
            if e > 0.49 {
                sources.push((col, row, e));
            }
        }
    }
    sources.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());

    // Pick 5-15 river sources from the highest spots.
    let target_rivers = rng.gen_range(5..=15).min(sources.len());

    for source_idx in 0..target_rivers {
        // Pick from the top sources with some randomness (don't always pick the absolute highest).
        let pick = if sources.len() > target_rivers * 2 {
            rng.gen_range(0..sources.len().min(target_rivers * 3))
        } else {
            source_idx % sources.len()
        };
        let (start_col, start_row, _) = sources[pick];

        // Trace downhill to coast/ocean.
        let mut path: Vec<(i32, i32)> = Vec::new();
        let mut cur_col = start_col;
        let mut cur_row = start_row;
        let mut reached_water = false;
        let max_steps = (width + height) as usize; // Safety limit.

        for _ in 0..max_steps {
            let cur_i = tile_idx(cur_col, cur_row, width, height).unwrap();
            let cur_biome = tiles[cur_i].biome;

            // Rivers dissolve into the sea — don't mark ocean tiles with a river.
            if cur_biome == Biome::Ocean {
                reached_water = true;
                break;
            }

            path.push((cur_col, cur_row));

            // Coast is the mouth — keep it in the path but stop here.
            if cur_biome == Biome::Coast {
                reached_water = true;
                break;
            }

            // Find the neighbor with the lowest elevation (steepest descent).
            let mut best_col = -1;
            let mut best_row = -1;
            let mut best_elev = tiles[cur_i].elevation;

            for (nc, nr) in hex_neighbors(cur_col, cur_row) {
                if let Some(ni) = tile_idx(nc, nr, width, height) {
                    let ne = tiles[ni].elevation;
                    if ne < best_elev {
                        best_elev = ne;
                        best_col = nc;
                        best_row = nr;
                    }
                }
            }

            // No downhill neighbor — we're in a depression. Stop.
            if best_col < 0 {
                break;
            }

            cur_col = best_col;
            cur_row = best_row;
        }

        if !reached_water || path.len() < 3 {
            continue;
        }

        // Mark river tiles along the path.
        for (i, &(rc, rr)) in path.iter().enumerate() {
            let ri = tile_idx(rc, rr, width, height).unwrap();
            // Rivers deepen as they flow downstream.
            let river_depth = if i < path.len() / 3 {
                1 // Headwaters
            } else if i < path.len() * 2 / 3 {
                2 // Midstream
            } else {
                3 // Lower river
            };
            tiles[ri].river = tiles[ri].river.max(river_depth);
        }
    }
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
