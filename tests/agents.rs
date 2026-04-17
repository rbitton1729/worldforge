use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use worldforge::agent::{alive_count, seed_agents, Agent};
use worldforge::world::World;
use worldforge::{run_simulation, SimConfig};

#[test]
fn agents_spawn_on_passable_land() {
    let w = World::generate(60, 30, 42);
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let agents = seed_agents(&w, 100, &mut rng);
    assert!(!agents.is_empty(), "should place at least one agent");
    for a in &agents {
        let tile = w
            .tile(a.col, a.row)
            .expect("agent coordinates must be in bounds");
        assert!(
            tile.biome.is_passable(),
            "agent on impassable biome {:?}",
            tile.biome
        );
        assert!(
            tile.biome.food_cap() > 0.0,
            "agent spawned on biome with no food capacity: {:?}",
            tile.biome
        );
    }
}

#[test]
fn alive_count_is_accurate() {
    let w = World::generate(60, 30, 11);
    let mut rng = ChaCha8Rng::seed_from_u64(11);
    let mut agents = seed_agents(&w, 20, &mut rng);
    let initial = agents.len();
    assert_eq!(alive_count(&agents), initial);

    agents[0].alive = false;
    agents[1].alive = false;
    assert_eq!(alive_count(&agents), initial - 2);

    for a in agents.iter_mut() {
        a.alive = false;
    }
    assert_eq!(alive_count(&agents), 0);
}

#[test]
fn agent_new_has_sane_defaults() {
    let a = Agent::new(7, "Test".to_string(), 3, 4, 800);
    assert_eq!(a.id, 7);
    assert_eq!(a.col, 3);
    assert_eq!(a.row, 4);
    assert!(a.alive);
    assert!(a.hunger >= 0.0 && a.hunger < 100.0);
    assert!(a.health > 0.0);
    assert_eq!(a.age, 0);
    assert!(a.settlement.is_none());
    // Roles are derived from skills, never assigned at construction.
    assert!(!a.is_warrior());
    assert!(!a.is_merchant());
    assert!(!a.is_traveling());
}

#[test]
fn agents_can_starve_on_barren_map() {
    // Tiny, harsh-populated map: lots of agents chasing scarce food eventually starve.
    let cfg = SimConfig {
        seed: 99,
        width: 20,
        height: 10,
        agents: 80,
        ticks: 400,
        chronicle_path: None,
    };
    let outcome = run_simulation(cfg);
    let dead = outcome.agents.iter().filter(|a| !a.alive).count();
    assert!(
        dead > 0,
        "expected some agents to die on a crowded barren map, got {} deaths",
        dead
    );
}

#[test]
fn agents_can_reproduce() {
    // Enough ticks + agents that reproduction has a reasonable chance.
    let cfg = SimConfig {
        seed: 2024,
        width: 80,
        height: 40,
        agents: 200,
        ticks: 300,
        chronicle_path: None,
    };
    let outcome = run_simulation(cfg);
    // Newborns get IDs >= initial seeded count; if len > initial, births occurred.
    assert!(
        outcome.agents.len() > 200,
        "expected population to grow via reproduction, total agents ever: {}",
        outcome.agents.len()
    );
}

#[test]
fn agents_die_of_old_age() {
    // Agents start staggered up to ~300 ticks old and live 600-900; long enough run
    // guarantees some natural deaths.
    let cfg = SimConfig {
        seed: 77,
        width: 80,
        height: 40,
        agents: 100,
        ticks: 700,
        chronicle_path: None,
    };
    let outcome = run_simulation(cfg);
    let old_age_deaths = outcome
        .agents
        .iter()
        .filter(|a| !a.alive && a.age >= a.max_age)
        .count();
    assert!(
        old_age_deaths > 0,
        "expected some deaths from old age after 700 ticks"
    );
}
