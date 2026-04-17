use worldforge::agent::alive_count;
use worldforge::{run_simulation, SimConfig};

#[test]
fn population_is_never_negative() {
    // alive_count returns usize so negativity is impossible, but make sure
    // it's consistent and bounded.
    let cfg = SimConfig {
        seed: 1,
        width: 40,
        height: 20,
        agents: 50,
        ticks: 200,
        chronicle_path: None,
    };
    let outcome = run_simulation(cfg);
    let alive = alive_count(&outcome.agents);
    assert!(alive <= outcome.agents.len());
}

#[test]
fn same_seed_same_final_population() {
    let base = SimConfig {
        seed: 314,
        width: 60,
        height: 30,
        agents: 100,
        ticks: 250,
        chronicle_path: None,
    };
    let a = run_simulation(SimConfig { ..base });
    let b = run_simulation(SimConfig {
        seed: 314,
        width: 60,
        height: 30,
        agents: 100,
        ticks: 250,
        chronicle_path: None,
    });
    assert_eq!(alive_count(&a.agents), alive_count(&b.agents));
    assert_eq!(a.agents.len(), b.agents.len());
    assert_eq!(a.settlements.list.len(), b.settlements.list.len());
    assert_eq!(a.final_tick, b.final_tick);
}

#[test]
fn simulation_terminates_at_requested_ticks() {
    let cfg = SimConfig {
        seed: 42,
        width: 60,
        height: 30,
        agents: 100,
        ticks: 50,
        chronicle_path: None,
    };
    let outcome = run_simulation(cfg);
    assert!(
        outcome.final_tick <= 50,
        "final_tick {} exceeded requested 50",
        outcome.final_tick
    );
}

#[test]
fn chronicle_output_is_nonempty() {
    let path = std::env::temp_dir().join("worldforge-chronicle-nonempty.txt");
    let cfg = SimConfig {
        seed: 42,
        width: 40,
        height: 20,
        agents: 50,
        ticks: 100,
        chronicle_path: Some(path.to_string_lossy().to_string()),
    };
    let _ = run_simulation(cfg);
    let contents = std::fs::read_to_string(&path).expect("chronicle file");
    assert!(!contents.is_empty(), "chronicle file should have content");
    assert!(
        contents.contains("worldforge"),
        "chronicle should contain prologue marker, got: {:?}",
        &contents[..contents.len().min(200)]
    );
}

#[test]
fn no_panic_on_tiny_map() {
    let cfg = SimConfig {
        seed: 1,
        width: 10,
        height: 10,
        agents: 5,
        ticks: 50,
        chronicle_path: None,
    };
    let _ = run_simulation(cfg);
}

#[test]
fn no_panic_on_zero_agents() {
    let cfg = SimConfig {
        seed: 1,
        width: 30,
        height: 15,
        agents: 0,
        ticks: 20,
        chronicle_path: None,
    };
    let outcome = run_simulation(cfg);
    assert_eq!(alive_count(&outcome.agents), 0);
    assert!(outcome.agents.is_empty());
}

#[test]
fn agent_health_and_hunger_bounded() {
    let cfg = SimConfig {
        seed: 99,
        width: 50,
        height: 25,
        agents: 80,
        ticks: 300,
        chronicle_path: None,
    };
    let outcome = run_simulation(cfg);
    for a in &outcome.agents {
        assert!(a.hunger >= 0.0, "hunger went negative: {}", a.hunger);
        assert!(a.hunger <= 100.0 + 1e-3, "hunger exceeded max: {}", a.hunger);
        // Health may dip below 0 briefly before the death check on the same tick
        // flips `alive`; once dead it isn't updated further. Dead agents may
        // legitimately have health <= 0.
        if a.alive {
            assert!(a.health > 0.0, "living agent with health {}", a.health);
            assert!(a.health <= 100.0 + 1e-3, "health exceeded max: {}", a.health);
        }
    }
}
