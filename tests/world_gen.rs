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
    for t in &w.tiles {
        assert!(
            t.food <= t.biome.food_cap() + 1e-4,
            "food {} exceeds cap {}",
            t.food,
            t.biome.food_cap()
        );
    }
}
