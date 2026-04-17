use worldforge::world::{Biome, World};
use worldforge::{run_simulation, SimConfig};

/// Find a plains tile at full fertility, not adjacent to a river (so the
/// river bonus doesn't perturb the test). Returns (col, row).
fn find_clean_plains(world: &World) -> Option<(i32, i32)> {
    for row in 0..world.height as i32 {
        for col in 0..world.width as i32 {
            if world.is_near_river(col, row) {
                continue;
            }
            if let Some(t) = world.tile(col, row) {
                if t.biome == Biome::Plains && t.fertility >= 0.999 {
                    return Some((col, row));
                }
            }
        }
    }
    None
}

#[test]
fn tile_has_fertility_in_range() {
    let w = World::generate(60, 30, 42);
    for t in &w.tiles {
        assert!(
            (0.0..=1.0).contains(&t.fertility),
            "fertility out of range: {} on {:?}",
            t.fertility,
            t.biome
        );
    }
}

#[test]
fn fertile_biomes_start_at_natural_max() {
    let w = World::generate(80, 40, 42);
    for t in &w.tiles {
        let expected = t.biome.natural_fertility();
        assert!(
            (t.fertility - expected).abs() < 1e-6,
            "biome {:?} fertility {} != natural {}",
            t.biome,
            t.fertility,
            expected
        );
    }
}

#[test]
fn depleted_tiles_produce_less_food() {
    let mut w = World::generate(60, 30, 7);

    // Find two clean plains tiles to compare side by side.
    let mut candidates: Vec<(i32, i32)> = Vec::new();
    for row in 0..w.height as i32 {
        for col in 0..w.width as i32 {
            if w.is_near_river(col, row) {
                continue;
            }
            if let Some(t) = w.tile(col, row) {
                if t.biome == Biome::Plains && t.fertility >= 0.999 {
                    candidates.push((col, row));
                    if candidates.len() >= 2 {
                        break;
                    }
                }
            }
        }
        if candidates.len() >= 2 {
            break;
        }
    }
    assert!(candidates.len() >= 2, "need two clean plains tiles");
    let (ac, ar) = candidates[0];
    let (bc, br) = candidates[1];

    // Drain both tiles; leave one fertile and deplete the other.
    if let Some(t) = w.tile_mut(ac, ar) {
        t.food = 0.0;
        t.fertility = 1.0;
    }
    if let Some(t) = w.tile_mut(bc, br) {
        t.food = 0.0;
        t.fertility = 0.4;
    }

    // Summer tick — a healthy tile should out-regenerate a depleted one.
    w.regen_food(30);

    let a_food = w.tile(ac, ar).unwrap().food;
    let b_food = w.tile(bc, br).unwrap().food;
    assert!(
        a_food > b_food,
        "healthier tile should regenerate more food: a={} b={}",
        a_food,
        b_food
    );
}

#[test]
fn depleted_tile_cap_respects_floor() {
    let mut w = World::generate(60, 30, 11);
    let (c, r) = find_clean_plains(&w).expect("clean plains tile");
    // Push fertility to zero and let the tile regenerate for many ticks.
    if let Some(t) = w.tile_mut(c, r) {
        t.fertility = 0.0;
        t.food = 0.0;
    }
    // Regen for a handful of summer ticks, but immediately drain fertility
    // back to zero each tick so only the floor'd cap matters.
    for _ in 0..200 {
        if let Some(t) = w.tile_mut(c, r) {
            t.fertility = 0.0;
        }
        w.regen_food(30);
    }
    let t = w.tile(c, r).unwrap();
    let base_cap = t.biome.food_cap();
    // Even at zero fertility, the tile should be allowed to hold up to 20% of base.
    assert!(
        t.food <= base_cap * 0.2 + 1e-3,
        "depleted-floor cap exceeded: food={} base_cap={}",
        t.food,
        base_cap
    );
}

#[test]
fn fertility_recovers_when_unharvested() {
    let mut w = World::generate(60, 30, 3);
    let (c, r) = find_clean_plains(&w).expect("clean plains tile");
    if let Some(t) = w.tile_mut(c, r) {
        t.fertility = 0.2;
    }
    // Run across a few full seasons so recovery sees varied season factors.
    for tick in 0..400u64 {
        w.regen_food(tick);
    }
    let fert = w.tile(c, r).unwrap().fertility;
    assert!(fert > 0.2, "fertility should recover, got {}", fert);
    assert!(fert <= 1.0 + 1e-6, "fertility exceeded 1.0: {}", fert);
}

#[test]
fn fertility_cannot_exceed_natural_cap() {
    let mut w = World::generate(60, 30, 5);
    // Find a hills tile (natural cap 0.8).
    let mut target = None;
    for row in 0..w.height as i32 {
        for col in 0..w.width as i32 {
            if let Some(t) = w.tile(col, row) {
                if t.biome == Biome::Hills {
                    target = Some((col, row));
                    break;
                }
            }
        }
        if target.is_some() {
            break;
        }
    }
    let (c, r) = target.expect("hills tile");
    // Drop fertility low and let it recover.
    if let Some(t) = w.tile_mut(c, r) {
        t.fertility = 0.1;
    }
    for tick in 0..5000u64 {
        w.regen_food(tick);
    }
    let fert = w.tile(c, r).unwrap().fertility;
    assert!(
        fert <= Biome::Hills.natural_fertility() + 1e-4,
        "hills fertility exceeded natural cap: {}",
        fert
    );
}

#[test]
fn simulation_keeps_fertility_bounded() {
    let cfg = SimConfig {
        seed: 2024,
        width: 80,
        height: 40,
        agents: 200,
        ticks: 400,
        chronicle_path: None,
    };
    let outcome = run_simulation(cfg);
    for t in &outcome.world.tiles {
        assert!(
            (0.0..=1.0).contains(&t.fertility),
            "fertility out of bounds after sim: {} on {:?}",
            t.fertility,
            t.biome
        );
        assert!(
            t.fertility <= t.biome.natural_fertility() + 1e-4,
            "fertility {} exceeds natural cap {} for {:?}",
            t.fertility,
            t.biome.natural_fertility(),
            t.biome
        );
    }
}

#[test]
fn foraging_depletes_land() {
    // Busy map with hungry agents should leave at least some plains/forest
    // measurably below their natural 1.0 baseline by the end of a long run.
    // Recovery is steady, so depletion shows up as a small but real gap.
    let cfg = SimConfig {
        seed: 2024,
        width: 80,
        height: 40,
        agents: 200,
        ticks: 400,
        chronicle_path: None,
    };
    let outcome = run_simulation(cfg);
    let min_fertile_biome = outcome
        .world
        .tiles
        .iter()
        .filter(|t| matches!(t.biome, Biome::Plains | Biome::Forest))
        .map(|t| t.fertility)
        .fold(f32::INFINITY, f32::min);
    assert!(
        min_fertile_biome < 1.0 - 1e-3,
        "expected foraging to leave at least one plains/forest tile below its natural cap, min={}",
        min_fertile_biome
    );
}
