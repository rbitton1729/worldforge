use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use worldforge::agent::{
    Agent, FORAGING_GROWTH, MERCHANT_DISPATCH_THRESHOLD, ROLE_RECOGNITION_THRESHOLD,
    SEED_FIGHTING_BIAS_MAX, SEED_FIGHTING_BIAS_MIN, SEED_SKILL_BIAS_MAX, SKILL_BASELINE,
    SKILL_DECAY, TRADING_GROWTH, WARRIOR_RECOGNITION_THRESHOLD, seed_agents, step_agents,
};
use worldforge::chronicle::Chronicle;
use worldforge::settlement::{Settlements, update_settlements};
use worldforge::world::{Biome, World};
use worldforge::{SimConfig, run_simulation};

fn sink_chronicle(tag: &str) -> Chronicle {
    Chronicle::to_file(
        &std::env::temp_dir()
            .join(format!("worldforge-skills-{}.txt", tag))
            .to_string_lossy(),
    )
    .expect("sink chronicle")
}

#[test]
fn newborn_skills_start_at_baseline() {
    let a = Agent::new(0, "T".into(), 0, 0, 800);
    assert_eq!(a.skills.fighting, SKILL_BASELINE);
    assert_eq!(a.skills.foraging, SKILL_BASELINE);
    assert_eq!(a.skills.trading, SKILL_BASELINE);
}

#[test]
fn seeded_agents_have_skills_in_bias_band() {
    // Founders get a small random bias above baseline (representing prior life
    // experience — DESIGN.md "skill bias"). Newborns are tested separately.
    let w = World::generate(60, 30, 7);
    let mut rng = ChaCha8Rng::seed_from_u64(7);
    let agents = seed_agents(&w, 200, &mut rng);
    let upper_generic = SKILL_BASELINE + SEED_SKILL_BIAS_MAX;
    let fighting_lower = SKILL_BASELINE + SEED_FIGHTING_BIAS_MIN;
    let fighting_upper = SKILL_BASELINE + SEED_FIGHTING_BIAS_MAX;
    for a in &agents {
        assert!(
            (fighting_lower - 1e-6..=fighting_upper + 1e-6).contains(&a.skills.fighting),
            "seed fighting {} out of [{}, {}]",
            a.skills.fighting,
            fighting_lower,
            fighting_upper
        );
        for v in [a.skills.foraging, a.skills.trading] {
            assert!(
                (SKILL_BASELINE..=upper_generic + 1e-6).contains(&v),
                "seed skill {} out of [{}, {}]",
                v,
                SKILL_BASELINE,
                upper_generic
            );
        }
        // Founders are below merchant recognition at seed. Warrior recognition
        // sits at 0.3 deliberately — the founder fighting bias straddles it so
        // a meaningful fraction of seeds are warriors from day one, which is
        // what bootstraps raids (and thus further fighting-skill growth).
        assert!(!a.is_merchant());
    }
    // At least some founders should cross the dispatch threshold so trade can
    // bootstrap. With 200 seeds and bias up to 0.3, this is statistically
    // overwhelming — but still worth asserting.
    let any_dispatchable = agents
        .iter()
        .any(|a| a.skills.trading > MERCHANT_DISPATCH_THRESHOLD);
    assert!(
        any_dispatchable,
        "expected at least one founder above MERCHANT_DISPATCH_THRESHOLD"
    );
}

#[test]
fn no_coin_flip_roles_for_newborns_in_long_run() {
    // After many ticks, any warriors/merchants present must have crossed the
    // recognition threshold via behavior — never via assignment.
    let cfg = SimConfig {
        seed: 314,
        width: 80,
        height: 40,
        agents: 200,
        ticks: 600,
        tick_rate: None,
        profile: false,
    };
    let outcome = run_simulation(cfg, &mut Chronicle::sink());
    for a in &outcome.agents {
        if a.is_warrior() {
            assert!(
                a.skills.fighting > WARRIOR_RECOGNITION_THRESHOLD,
                "warrior {} has fighting skill {} ≤ threshold",
                a.name,
                a.skills.fighting
            );
        }
        if a.is_merchant() {
            assert!(
                a.skills.trading > ROLE_RECOGNITION_THRESHOLD,
                "merchant {} has trading skill {} ≤ threshold",
                a.name,
                a.skills.trading
            );
        }
    }
}

#[test]
fn foraging_grows_when_an_agent_eats() {
    // Place a single hungry agent on a food-bearing tile, run one tick, and
    // verify the foraging skill ticked up. We confirm the agent ate by also
    // checking hunger fell.
    let mut w = World::generate(80, 40, 42);
    let mut found: Option<(i32, i32)> = None;
    'outer: for row in 1..(w.height as i32 - 1) {
        for col in 1..(w.width as i32 - 1) {
            if w.tile(col, row).map_or(false, |t| t.biome == Biome::Plains) {
                found = Some((col, row));
                break 'outer;
            }
        }
    }
    let (c, r) = found.expect("plains tile");
    if let Some(t) = w.tile_mut(c, r) {
        t.food = 10.0;
    }

    let mut a = Agent::new(0, "T".into(), c, r, 1000);
    a.hunger = 60.0; // above the 30.0 nibble bar
    let before_skill = a.skills.foraging;
    let before_hunger = a.hunger;

    let mut agents = vec![a];
    let mut settlements = Settlements::new();
    let mut rng = ChaCha8Rng::seed_from_u64(1);
    let mut chronicle = sink_chronicle("forage-grows");
    step_agents(
        &mut agents,
        &mut w,
        &mut settlements,
        &mut rng,
        &mut chronicle,
        1,
    );

    let after = &agents[0];
    assert!(
        after.hunger < before_hunger,
        "agent should have eaten: hunger {} -> {}",
        before_hunger,
        after.hunger
    );
    // Skill grew by FORAGING_GROWTH minus one tick of decay.
    let expected = (before_skill + FORAGING_GROWTH - SKILL_DECAY).max(0.0);
    assert!(
        (after.skills.foraging - expected).abs() < 1e-5,
        "expected foraging ~{}, got {}",
        expected,
        after.skills.foraging
    );
}

#[test]
fn skills_decay_over_time_when_unused() {
    // Place an agent on an impassable-to-food spot (mountains have food_cap=0)
    // so they can't gain foraging skill. Step many ticks and watch the skill
    // drift toward zero.
    let mut w = World::generate(80, 40, 42);
    let mut found: Option<(i32, i32)> = None;
    'outer: for row in 1..(w.height as i32 - 1) {
        for col in 1..(w.width as i32 - 1) {
            if w.tile(col, row)
                .map_or(false, |t| t.biome == Biome::Mountains)
            {
                // Make sure all neighbors are mountains too so the agent
                // can't wander off and find food.
                let nb = w.neighbors(col, row);
                let all_mtn = nb
                    .iter()
                    .all(|&(nc, nr)| w.tile(nc, nr).map_or(true, |t| t.biome == Biome::Mountains));
                if all_mtn {
                    found = Some((col, row));
                    break 'outer;
                }
            }
        }
    }
    // Fall back: zero out all food on a small region so the agent can't eat.
    let (c, r) = found.unwrap_or_else(|| {
        let cc = (w.width / 2) as i32;
        let rr = (w.height / 2) as i32;
        for dr in -10..=10i32 {
            for dc in -10..=10i32 {
                if let Some(t) = w.tile_mut(cc + dc, rr + dr) {
                    t.food = 0.0;
                    t.fertility = 0.0;
                }
            }
        }
        (cc, rr)
    });

    let mut a = Agent::new(0, "T".into(), c, r, 100_000);
    a.skills.foraging = 0.5;
    a.skills.fighting = 0.5;
    a.skills.trading = 0.5;
    let mut agents = vec![a];
    let mut settlements = Settlements::new();
    let mut rng = ChaCha8Rng::seed_from_u64(1);
    let mut chronicle = sink_chronicle("decay");
    for tick in 1..=200u64 {
        step_agents(
            &mut agents,
            &mut w,
            &mut settlements,
            &mut rng,
            &mut chronicle,
            tick,
        );
        if !agents[0].alive {
            break;
        }
    }
    let after = &agents[0];
    // Either the agent died (still proves decay was applied for many ticks)
    // or they're alive with reduced skills. Check decay is real either way.
    assert!(
        after.skills.fighting < 0.5,
        "fighting should have decayed: {}",
        after.skills.fighting
    );
    assert!(
        after.skills.trading < 0.5,
        "trading should have decayed: {}",
        after.skills.trading
    );
}

#[test]
fn settlement_does_not_dispatch_without_skilled_trader() {
    // Hand-build a settlement and verify update_settlements doesn't load
    // cargo on any loyal agent because all skills are at baseline (0.1)
    // which is below MERCHANT_DISPATCH_THRESHOLD (0.3).
    let w = World::generate(80, 40, 7);
    // Find a plains tile we can plant a settlement on.
    let mut origin: Option<(i32, i32)> = None;
    'outer: for row in 5..(w.height as i32 - 5) {
        for col in 5..(w.width as i32 - 5) {
            if w.tile(col, row).map_or(false, |t| t.biome == Biome::Plains) {
                origin = Some((col, row));
                break 'outer;
            }
        }
    }
    let (oc, or) = origin.expect("plains tile");

    // Cluster enough agents at this spot to found a settlement, then run one
    // update_settlements tick.
    let mut agents: Vec<Agent> = (0..6)
        .map(|i| Agent::new(i, format!("T{}", i), oc, or, 10_000))
        .collect();
    let mut settlements = Settlements::new();
    let mut rng = ChaCha8Rng::seed_from_u64(1);
    let mut chronicle = sink_chronicle("dispatch-noskill");
    update_settlements(
        &mut settlements,
        &mut agents,
        &w,
        &mut rng,
        &mut chronicle,
        1,
    );
    assert!(
        !settlements.list.is_empty(),
        "expected the cluster to found a settlement"
    );
    let sid = settlements.list[0].id;

    // Pump in stockpile from outside (simulating a season of foraging) and
    // try to dispatch — we also need a destination settlement to dispatch to,
    // so plant a second one across the map.
    if let Some(s) = settlements.list.iter_mut().find(|s| s.id == sid) {
        s.stockpile = 50.0;
    }
    // Spawn a second cluster far away.
    let mut far_origin: Option<(i32, i32)> = None;
    'outer2: for row in 5..(w.height as i32 - 5) {
        for col in 5..(w.width as i32 - 5) {
            if (col - oc).abs() + (row - or).abs() < 12 {
                continue;
            }
            if w.tile(col, row).map_or(false, |t| t.biome == Biome::Plains) {
                far_origin = Some((col, row));
                break 'outer2;
            }
        }
    }
    let (fc, fr) = far_origin.expect("far plains tile");
    for i in 6..12 {
        agents.push(Agent::new(i, format!("F{}", i), fc, fr, 10_000));
    }
    update_settlements(
        &mut settlements,
        &mut agents,
        &w,
        &mut rng,
        &mut chronicle,
        2,
    );
    if let Some(s) = settlements.list.iter_mut().find(|s| s.id == sid) {
        s.stockpile = 50.0;
    }

    // Now run a tick — no agent has trading > 0.3, so no cargo should be loaded.
    update_settlements(
        &mut settlements,
        &mut agents,
        &w,
        &mut rng,
        &mut chronicle,
        3,
    );
    let any_traveling = agents
        .iter()
        .any(|a| a.alive && a.settlement == Some(sid) && a.is_traveling());
    assert!(
        !any_traveling,
        "no agent should be dispatched when nobody crosses MERCHANT_DISPATCH_THRESHOLD ({})",
        MERCHANT_DISPATCH_THRESHOLD
    );
}

#[test]
fn settlement_dispatches_skilled_trader_when_one_exists() {
    let w = World::generate(80, 40, 7);
    let mut origin: Option<(i32, i32)> = None;
    'outer: for row in 5..(w.height as i32 - 5) {
        for col in 5..(w.width as i32 - 5) {
            if w.tile(col, row).map_or(false, |t| t.biome == Biome::Plains) {
                origin = Some((col, row));
                break 'outer;
            }
        }
    }
    let (oc, or) = origin.expect("plains tile");

    // Found settlement A.
    let mut agents: Vec<Agent> = (0..6)
        .map(|i| Agent::new(i, format!("T{}", i), oc, or, 10_000))
        .collect();
    let mut settlements = Settlements::new();
    let mut rng = ChaCha8Rng::seed_from_u64(1);
    let mut chronicle = sink_chronicle("dispatch-skilled");
    update_settlements(
        &mut settlements,
        &mut agents,
        &w,
        &mut rng,
        &mut chronicle,
        1,
    );
    let sid = settlements.list[0].id;

    // Found settlement B far away as a destination.
    let mut far_origin: Option<(i32, i32)> = None;
    'outer2: for row in 5..(w.height as i32 - 5) {
        for col in 5..(w.width as i32 - 5) {
            if (col - oc).abs() + (row - or).abs() < 12 {
                continue;
            }
            if w.tile(col, row).map_or(false, |t| t.biome == Biome::Plains) {
                far_origin = Some((col, row));
                break 'outer2;
            }
        }
    }
    let (fc, fr) = far_origin.expect("far plains tile");
    for i in 6..12 {
        agents.push(Agent::new(i, format!("F{}", i), fc, fr, 10_000));
    }
    update_settlements(
        &mut settlements,
        &mut agents,
        &w,
        &mut rng,
        &mut chronicle,
        2,
    );
    assert!(
        settlements.alive_count() >= 2,
        "expected two settlements, got {}",
        settlements.alive_count()
    );

    // Pump up one loyal agent's trading skill above threshold + give the
    // settlement a surplus.
    for a in agents.iter_mut() {
        if a.settlement == Some(sid) {
            a.skills.trading = TRADING_GROWTH + MERCHANT_DISPATCH_THRESHOLD + 0.05; // > 0.3
            break;
        }
    }
    if let Some(s) = settlements.list.iter_mut().find(|s| s.id == sid) {
        s.stockpile = 50.0;
    }

    update_settlements(
        &mut settlements,
        &mut agents,
        &w,
        &mut rng,
        &mut chronicle,
        3,
    );
    let any_traveling = agents.iter().any(|a| a.alive && a.is_traveling());
    assert!(
        any_traveling,
        "expected the skilled agent to be dispatched as merchant"
    );
}
