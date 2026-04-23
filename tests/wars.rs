use worldforge::chronicle::Chronicle;
use worldforge::{SimConfig, run_simulation};

fn run_and_read(seed: u64, ticks: u64, path_suffix: &str) -> String {
    let path = std::env::temp_dir().join(format!("worldforge-chronicle-{}.txt", path_suffix));
    let cfg = SimConfig {
        seed,
        width: 60,
        height: 30,
        agents: 200,
        ticks,
        tick_rate: None,
        profile: false,
    };
    let mut chronicle = Chronicle::to_file(path.to_str().unwrap()).unwrap();
    run_simulation(cfg, &mut chronicle);
    drop(chronicle);
    std::fs::read_to_string(&path).expect("chronicle readable")
}

fn run(seed: u64, ticks: u64) -> worldforge::SimOutcome {
    let cfg = SimConfig {
        seed,
        width: 60,
        height: 30,
        agents: 200,
        ticks,
        tick_rate: None,
        profile: false,
    };
    let mut chronicle = Chronicle::sink();
    run_simulation(cfg, &mut chronicle)
}

/// War chronicles should appear in long-running simulations with enough
/// agents and settlements to generate raids, blood feuds, and wars.
#[test]
fn war_chronicles_appear_in_long_sim() {
    let text = run_and_read(77, 4000, "wars");

    // Blood feud is the trigger for war declaration.
    assert!(
        text.contains("blood feud"),
        "expected at least one blood feud in a long simulation; got:\n{}",
        &text[..text.len().min(2000)]
    );

    // War break-out line.
    assert!(
        text.contains("War breaks out between"),
        "expected war declaration in chronicle"
    );

    // Named battle or heavy battle line.
    assert!(
        text.contains("Battle of") || text.contains("rages on"),
        "expected named battles or 'war rages on' milestone"
    );
}

/// After a conquest, no living agent should still belong to the dead settlement.
/// This also exercises the war-ending chronicle line.
#[test]
fn war_ends_with_conquest_or_peace() {
    let text = run_and_read(88, 5000, "war-end");
    let outcome = run(88, 5000);

    // At least one war should have ended.
    let war_ended = text.contains("ends with")
        || text.contains("sues for peace");
    assert!(
        war_ended,
        "expected at least one war to end via conquest or peace; chronicle:\n{}",
        &text[..text.len().min(2000)]
    );

    // Post-conquest population sanity: dead settlements have no loyal agents.
    let dead_ids: Vec<u32> = outcome
        .settlements
        .list
        .iter()
        .filter(|s| !s.alive)
        .map(|s| s.id)
        .collect();
    for a in outcome.agents.iter().filter(|a| a.alive) {
        if let Some(sid) = a.settlement {
            assert!(
                !dead_ids.contains(&sid),
                "agent {} still loyal to dead settlement {}",
                a.name,
                sid
            );
        }
    }
}

/// Settlement names must be unique — no two alive or dead settlements share
/// a name. This exercises the name-dedup retry logic in `found()`.
#[test]
fn settlement_names_are_unique() {
    let outcome = run(99, 3000);
    let mut names: Vec<&str> = outcome
        .settlements
        .list
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    names.sort();
    for window in names.windows(2) {
        assert_ne!(
            window[0], window[1],
            "duplicate settlement name: {}",
            window[0]
        );
    }
}

/// Alliances should form from sustained trade between two settlements.
/// After enough trade trips, the route becomes allied.
#[test]
fn alliances_form_from_trade() {
    let outcome = run(123, 3000);

    let allied_routes: usize = outcome
        .settlements
        .list
        .iter()
        .filter(|s| s.alive)
        .map(|s| s.routes.iter().filter(|r| r.allied).count())
        .sum();

    assert!(
        allied_routes > 0,
        "expected at least one allied trade route to form in a long sim"
    );
}

/// Wars can involve multiple battles before ending. The chronicle records each
/// heavy clash as a "Battle of X" line, and the "rages on" milestone fires
/// once a war reaches 3+ battles. We verify there are more total battles than
/// wars, which proves at least one war lasted multiple engagements.
#[test]
fn war_tracks_multiple_battles() {
    let text = run_and_read(77, 8000, "multi-battle");

    let wars_started = text.matches("War breaks out between").count();
    let battles = text.matches("Battle of").count();

    assert!(
        wars_started > 0,
        "expected at least one war to start"
    );
    assert!(
        battles > wars_started,
        "expected more battles ({}) than wars ({}), proving some wars lasted multiple clashes",
        battles, wars_started
    );
}
