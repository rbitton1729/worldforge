use worldforge::world::{CLIMATE_DRIFT_MAX, CLIMATE_DRIFT_MIN, World};

#[test]
fn climate_drift_stays_in_range_over_2000_ticks() {
    let mut w = World::generate(60, 30, 42);
    for tick in 1..=2000u64 {
        let _ = w.tick_climate(tick);
        assert!(
            (CLIMATE_DRIFT_MIN..=CLIMATE_DRIFT_MAX).contains(&w.climate_drift),
            "climate_drift out of range at tick {}: {}",
            tick,
            w.climate_drift
        );
    }
}

#[test]
fn same_seed_same_climate_pattern() {
    let mut a = World::generate(60, 30, 314);
    let mut b = World::generate(60, 30, 314);
    for tick in 1..=3000u64 {
        let ea = a.tick_climate(tick);
        let eb = b.tick_climate(tick);
        assert_eq!(ea, eb, "climate events diverged at tick {}", tick);
        assert_eq!(
            a.climate_drift.to_bits(),
            b.climate_drift.to_bits(),
            "drift diverged at tick {}: {} vs {}",
            tick,
            a.climate_drift,
            b.climate_drift
        );
    }
}

#[test]
fn different_seeds_drift_differently() {
    let mut a = World::generate(60, 30, 1);
    let mut b = World::generate(60, 30, 2);
    for tick in 1..=2000u64 {
        a.tick_climate(tick);
        b.tick_climate(tick);
    }
    assert!(
        (a.climate_drift - b.climate_drift).abs() > 1e-6,
        "different seeds produced identical drift: {}",
        a.climate_drift
    );
}

#[test]
fn tick_climate_no_op_off_year_boundary() {
    let mut w = World::generate(40, 20, 9);
    let baseline = w.climate_drift;
    // Ticks that aren't year boundaries should not change drift.
    for tick in 1..100u64 {
        assert!(w.tick_climate(tick).is_none());
        assert_eq!(w.climate_drift.to_bits(), baseline.to_bits());
    }
    // Year boundary moves it.
    let _ = w.tick_climate(100);
    assert!(
        (w.climate_drift - baseline).abs() > 0.0,
        "drift should advance on year boundary"
    );
}
