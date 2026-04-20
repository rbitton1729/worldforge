use worldforge::chronicle::Chronicle;
use worldforge::world::World;
use worldforge::{SimConfig, run_simulation};

#[test]
fn every_region_has_nonempty_name() {
    for seed in [1u64, 42, 314, 7, 100] {
        let w = World::generate(80, 40, seed);
        for r in &w.regions {
            assert!(
                !r.name.trim().is_empty(),
                "seed {}: region at {:?} has empty name",
                seed,
                r.center
            );
        }
    }
}

#[test]
fn regions_do_not_overlap() {
    let w = World::generate(80, 40, 42);
    let mut counted = vec![0usize; w.regions.len()];
    for row in 0..w.height as i32 {
        for col in 0..w.width as i32 {
            if let Some(r) = w.region_at(col, row) {
                let idx = w.regions.iter().position(|rr| std::ptr::eq(rr, r)).unwrap();
                counted[idx] += 1;
            }
        }
    }
    for (i, r) in w.regions.iter().enumerate() {
        assert_eq!(
            counted[i], r.tile_count,
            "region {} ({}) has {} members by lookup but tile_count = {}",
            i, r.name, counted[i], r.tile_count
        );
    }
}

#[test]
fn region_count_is_reasonable() {
    for seed in [1u64, 42, 314, 7] {
        let w = World::generate(80, 40, seed);
        let n = w.regions.len();
        assert!(
            (3..=20).contains(&n),
            "seed {}: expected 3-20 regions, got {}",
            seed,
            n
        );
    }
}

#[test]
fn region_names_are_unique() {
    let w = World::generate(80, 40, 42);
    let mut seen = std::collections::HashSet::new();
    for r in &w.regions {
        assert!(
            seen.insert(r.name.clone()),
            "duplicate region name: {}",
            r.name
        );
    }
}

#[test]
fn named_regions_appear_in_settlement_founding() {
    let mut found_in_region = false;
    for seed in [1u64, 42, 7, 100, 314, 999] {
        let path = std::env::temp_dir().join(format!("worldforge-region-founding-{}.txt", seed));
        let cfg = SimConfig {
            seed,
            width: 80,
            height: 40,
            agents: 300,
            ticks: 120,
            tick_rate: None,
            profile: false,
        };
        let mut chronicle = Chronicle::to_file(path.to_str().unwrap()).unwrap();
        let outcome = run_simulation(cfg, &mut chronicle);
        drop(chronicle);
        let content = std::fs::read_to_string(&path).unwrap();
        for line in content.lines() {
            if line.contains("settlers gathers in ")
                && outcome.world.regions.iter().any(|r| line.contains(&r.name))
            {
                found_in_region = true;
                break;
            }
        }
        if found_in_region {
            break;
        }
    }
    assert!(
        found_in_region,
        "no settlement founding event mentions a region name"
    );
}
