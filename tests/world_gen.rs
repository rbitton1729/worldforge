use worldforge::world::{Biome, World};

#[test]
fn world_has_correct_dimensions() {
    let w = World::generate(80, 40, 42);
    assert_eq!(w.width, 80);
    assert_eq!(w.height, 40);
    assert_eq!(w.tiles.len(), 80 * 40);
}

#[test]
fn world_has_ocean_and_land() {
    let w = World::generate(80, 40, 42);
    let mut has_ocean = false;
    let mut has_land = false;
    for t in &w.tiles {
        if t.biome == Biome::Ocean {
            has_ocean = true;
        }
        if t.biome.is_passable() && t.biome != Biome::Ocean {
            has_land = true;
        }
    }
    assert!(has_ocean, "world should contain ocean tiles");
    assert!(has_land, "world should contain passable land tiles");
}

#[test]
fn food_values_are_nonnegative() {
    let w = World::generate(60, 30, 7);
    for t in &w.tiles {
        assert!(t.food >= 0.0, "food should be non-negative, got {}", t.food);
        assert!(
            t.food <= t.biome.food_cap() + f32::EPSILON,
            "food {} exceeds cap {} for biome {:?}",
            t.food,
            t.biome.food_cap(),
            t.biome
        );
    }
}

#[test]
fn same_seed_produces_identical_world() {
    let a = World::generate(50, 30, 12345);
    let b = World::generate(50, 30, 12345);
    assert_eq!(a.tiles.len(), b.tiles.len());
    for (ta, tb) in a.tiles.iter().zip(b.tiles.iter()) {
        assert_eq!(ta.biome, tb.biome);
        assert_eq!(ta.elevation.to_bits(), tb.elevation.to_bits());
        assert_eq!(ta.temperature.to_bits(), tb.temperature.to_bits());
        assert_eq!(ta.moisture.to_bits(), tb.moisture.to_bits());
        assert_eq!(ta.food.to_bits(), tb.food.to_bits());
    }
}

#[test]
fn different_seeds_produce_different_worlds() {
    let a = World::generate(50, 30, 1);
    let b = World::generate(50, 30, 2);
    let diffs = a
        .tiles
        .iter()
        .zip(b.tiles.iter())
        .filter(|(ta, tb)| ta.biome != tb.biome)
        .count();
    assert!(diffs > 0, "different seeds should produce different biomes");
}

#[test]
fn regen_food_respects_cap() {
    let mut w = World::generate(40, 20, 3);
    for _ in 0..500 {
        w.regen_food(30); // summer: regen factor 1.6
    }
    for row in 0..w.height as i32 {
        for col in 0..w.width as i32 {
            let i = w.idx(col, row).unwrap();
            let t = &w.tiles[i];
            // Tiles adjacent to a river hold 50% more food than their biome base.
            let cap = if w.is_near_river(col, row) {
                t.biome.food_cap() * 1.5
            } else {
                t.biome.food_cap()
            };
            assert!(t.food <= cap + 1e-4, "food {} exceeds cap {}", t.food, cap);
        }
    }
}

#[test]
fn rivers_within_reasonable_count() {
    for seed in [1u64, 42, 314, 999] {
        let w = World::generate(80, 40, seed);
        let n = w.river_count();
        assert!(
            (1..=20).contains(&n),
            "seed {}: expected 1-20 rivers, got {}",
            seed,
            n
        );
    }
}

#[test]
fn every_river_reaches_water() {
    let w = World::generate(80, 40, 42);
    let mut visited = vec![false; w.tiles.len()];
    for row in 0..w.height as i32 {
        for col in 0..w.width as i32 {
            let seed_i = w.idx(col, row).unwrap();
            if visited[seed_i] || w.tiles[seed_i].river == 0 {
                continue;
            }
            let mut stack = vec![(col, row)];
            let mut size = 0usize;
            let mut reaches = false;
            while let Some((c, r)) = stack.pop() {
                let idx = match w.idx(c, r) {
                    Some(i) => i,
                    None => continue,
                };
                if visited[idx] || w.tiles[idx].river == 0 {
                    continue;
                }
                visited[idx] = true;
                size += 1;
                if w.tiles[idx].biome == Biome::Coast {
                    reaches = true;
                }
                for (nc, nr) in w.neighbors(c, r) {
                    if let Some(ni) = w.idx(nc, nr) {
                        if w.tiles[ni].biome == Biome::Ocean {
                            reaches = true;
                        }
                    }
                    stack.push((nc, nr));
                }
            }
            assert!(
                reaches,
                "river of {} tiles starting at ({}, {}) never reaches water",
                size, col, row
            );
        }
    }
}

#[test]
fn ocean_tiles_have_no_river() {
    for seed in [1u64, 42, 314] {
        let w = World::generate(80, 40, seed);
        for t in &w.tiles {
            if t.biome == Biome::Ocean {
                assert_eq!(
                    t.river, 0,
                    "seed {}: ocean tile marked with river depth {}",
                    seed, t.river
                );
            }
        }
    }
}
