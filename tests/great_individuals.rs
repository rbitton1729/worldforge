use worldforge::agent::{Agent, Deeds, choose_epithet};
use worldforge::chronicle::Chronicle;
use worldforge::{SimConfig, run_simulation};

#[test]
fn deeds_default_is_not_notable() {
    let d = Deeds::new();
    assert!(!d.is_notable());
}

#[test]
fn two_raids_led_is_notable() {
    let mut d = Deeds::new();
    d.raids_led = 2;
    assert!(d.is_notable());
}

#[test]
fn three_deliveries_is_notable() {
    let mut d = Deeds::new();
    d.deliveries = 3;
    assert!(d.is_notable());
}

#[test]
fn survived_sack_is_notable() {
    let mut d = Deeds::new();
    d.survived_sack = true;
    assert!(d.is_notable());
}

#[test]
fn founder_with_two_defenses_is_notable() {
    let mut d = Deeds::new();
    d.founded_settlement = true;
    d.defenses = 2;
    assert!(d.is_notable());
}

#[test]
fn founder_without_defenses_is_not_notable() {
    let mut d = Deeds::new();
    d.founded_settlement = true;
    d.defenses = 1;
    assert!(!d.is_notable());
}

#[test]
fn epithet_warlord_wins_over_merchant() {
    let mut d = Deeds::new();
    d.raids_led = 2;
    d.deliveries = 5;
    let e = choose_epithet(&d, 0);
    assert!(
        e.contains("Conqueror")
            || e.contains("Iron")
            || e.contains("Fierce")
            || e.contains("Raider")
            || e.contains("Unyielding"),
        "expected warlord epithet, got: {}",
        e
    );
}

#[test]
fn epithet_deterministic_by_id() {
    let d = Deeds::new();
    // Default fallback
    let e1 = choose_epithet(&d, 42);
    let e2 = choose_epithet(&d, 42);
    assert_eq!(e1, e2);
}

#[test]
fn agent_new_has_no_epithet() {
    let a = Agent::new(0, "Test".into(), 0, 0, 800);
    assert!(a.epithet.is_none());
    assert!(!a.deeds.is_notable());
}

#[test]
fn chronicle_great_individuals_appear_in_long_sim() {
    // Run a long sim to give enough time for deeds to accumulate.
    // At least some agents should become notable.
    let path = std::env::temp_dir().join("worldforge-great-individuals.txt");
    let cfg = SimConfig {
        seed: 2024,
        width: 80,
        height: 40,
        agents: 200,
        ticks: 2000,
        tick_rate: None,
        profile: false,
    };
    let mut chronicle = Chronicle::to_file(path.to_str().unwrap()).unwrap();
    let _outcome = run_simulation(cfg, &mut chronicle);
    drop(chronicle);
    let text = std::fs::read_to_string(&path).unwrap();

    // Check that at least one great individual was recognized (lines with *** and an epithet).
    let great_lines: Vec<&str> = text
        .lines()
        .filter(|l| {
            l.contains("***")
                && (l.contains("who led")
                    || l.contains("who carried")
                    || l.contains("who survived")
                    || l.contains("who founded")
                    || l.contains("whose deeds"))
        })
        .collect();
    assert!(
        !great_lines.is_empty(),
        "expected at least one great individual in 2000-tick sim, got none. Chronicle excerpt:\n{}",
        &text[text.len().saturating_sub(2000)..]
    );
}

#[test]
fn great_individuals_deterministic() {
    let cfg = SimConfig {
        seed: 314,
        width: 60,
        height: 30,
        agents: 100,
        ticks: 1000,
        tick_rate: None,
        profile: false,
    };

    let path_a = std::env::temp_dir().join("worldforge-gi-det-a.txt");
    let mut ch_a = Chronicle::to_file(path_a.to_str().unwrap()).unwrap();
    let _ = run_simulation(SimConfig { ..cfg }, &mut ch_a);
    drop(ch_a);

    let path_b = std::env::temp_dir().join("worldforge-gi-det-b.txt");
    let mut ch_b = Chronicle::to_file(path_b.to_str().unwrap()).unwrap();
    let _ = run_simulation(SimConfig { seed: 314, ..cfg }, &mut ch_b);
    drop(ch_b);

    let a = std::fs::read_to_string(&path_a).unwrap();
    let b = std::fs::read_to_string(&path_b).unwrap();
    assert_eq!(
        a, b,
        "same seed must produce identical great-individual chronicle"
    );
}
