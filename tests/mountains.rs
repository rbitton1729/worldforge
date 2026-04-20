use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use worldforge::agent::{Agent, MOUNTAIN_MOVE_HUNGER, step_agents};
use worldforge::chronicle::Chronicle;
use worldforge::settlement::Settlements;
use worldforge::world::{Biome, World};

#[test]
fn mountains_are_passable() {
    assert!(Biome::Mountains.is_passable());
}

#[test]
fn only_ocean_is_impassable() {
    for b in [
        Biome::Coast,
        Biome::Plains,
        Biome::Forest,
        Biome::Hills,
        Biome::Mountains,
        Biome::Desert,
        Biome::Tundra,
    ] {
        assert!(b.is_passable(), "{:?} should be passable", b);
    }
    assert!(!Biome::Ocean.is_passable());
}

#[test]
fn mountain_penalty_constant_is_positive() {
    assert!(MOUNTAIN_MOVE_HUNGER > 0.0);
}

fn sink_chronicle(tag: &str) -> Chronicle {
    Chronicle::to_file(
        &std::env::temp_dir()
            .join(format!("worldforge-{}.txt", tag))
            .to_string_lossy(),
    )
    .expect("sink chronicle")
}

/// Run one tick of step_agents with a single starving agent standing at
/// (sc, sr). The neighbor at (tc, tr) is the only tile carrying food within
/// range, so the agent's step_toward target is unambiguous. Returns the
/// post-tick hunger delta.
fn hunger_delta_stepping_onto(target_biome: Biome) -> f32 {
    let mut w = World::generate(80, 40, 42);

    // Find a plains tile with a plains neighbor we can repurpose as the target.
    let mut found: Option<((i32, i32), (i32, i32))> = None;
    'outer: for row in 1..(w.height as i32 - 1) {
        for col in 1..(w.width as i32 - 1) {
            if w.tile(col, row).map_or(false, |t| t.biome == Biome::Plains) {
                for (nc, nr) in w.neighbors(col, row) {
                    if w.tile(nc, nr).map_or(false, |t| t.biome == Biome::Plains) {
                        found = Some(((col, row), (nc, nr)));
                        break 'outer;
                    }
                }
            }
        }
    }
    let ((sc, sr), (tc, tr)) = found.expect("need a plains↔plains adjacency");

    // Drain food everywhere within radius 3 so the sole food source is the
    // target we're about to set up.
    for dr in -3..=3i32 {
        for dc in -3..=3i32 {
            if let Some(t) = w.tile_mut(sc + dc, sr + dr) {
                t.food = 0.0;
            }
        }
    }
    if let Some(t) = w.tile_mut(tc, tr) {
        t.biome = target_biome;
        t.food = 10.0;
    }

    let mut agent = Agent::new(0, "T".into(), sc, sr, 1_000);
    agent.hunger = 80.0;
    let before = agent.hunger;

    let mut agents = vec![agent];
    let mut settlements = Settlements::new();
    let mut rng = ChaCha8Rng::seed_from_u64(1);
    let mut chronicle = sink_chronicle(&format!("mtn-{:?}", target_biome));
    step_agents(
        &mut agents,
        &mut w,
        &mut settlements,
        &mut rng,
        &mut chronicle,
        1,
    );

    assert_eq!(
        (agents[0].col, agents[0].row),
        (tc, tr),
        "agent should have stepped onto the target {:?} tile",
        target_biome
    );
    agents[0].hunger - before
}

#[test]
fn moving_onto_mountain_costs_more_hunger_than_plains() {
    let plains_delta = hunger_delta_stepping_onto(Biome::Plains);
    let mountain_delta = hunger_delta_stepping_onto(Biome::Mountains);
    assert!(
        mountain_delta > plains_delta + 0.5,
        "mountain move should cost more hunger than plains: plains={} mountain={}",
        plains_delta,
        mountain_delta
    );
    assert!(
        (mountain_delta - plains_delta - MOUNTAIN_MOVE_HUNGER).abs() < 0.1,
        "mountain move delta should be plains delta + MOUNTAIN_MOVE_HUNGER: plains={} mountain={} penalty={}",
        plains_delta,
        mountain_delta,
        MOUNTAIN_MOVE_HUNGER
    );
}
