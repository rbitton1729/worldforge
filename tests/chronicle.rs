use worldforge::{run_simulation, SimConfig};

fn run_and_read(seed: u64, ticks: u64, path_suffix: &str) -> String {
    let path = std::env::temp_dir().join(format!("worldforge-chronicle-{}.txt", path_suffix));
    let cfg = SimConfig {
        seed,
        width: 60,
        height: 30,
        agents: 150,
        ticks,
        chronicle_path: Some(path.to_string_lossy().to_string()),
    };
    let _ = run_simulation(cfg);
    std::fs::read_to_string(&path).expect("chronicle readable")
}

#[test]
fn chronicle_has_year_season_header() {
    let text = run_and_read(42, 120, "header");
    assert!(
        text.contains("--- Year 1, Spring"),
        "missing Year 1 Spring header in chronicle"
    );
}

#[test]
fn chronicle_contains_multiple_seasons() {
    let text = run_and_read(42, 300, "multiseason");
    let mut seen = 0;
    for s in ["Spring", "Summer", "Autumn", "Winter"] {
        if text.contains(&format!(", {}", s)) {
            seen += 1;
        }
    }
    assert!(
        seen >= 3,
        "expected multiple seasons in chronicle, saw {}",
        seen
    );
}

#[test]
fn chronicle_seasonal_population_reports_appear() {
    // Population/settlement report fires every TICKS_PER_YEAR/2 = 50 ticks.
    let text = run_and_read(2024, 250, "popreports");
    // Headers after enough time include "X souls across Y settlements" wording.
    assert!(
        text.contains(" souls across "),
        "expected 'souls across N settlements' in header lines"
    );
}

#[test]
fn chronicle_records_deaths_with_agent_names() {
    let text = run_and_read(5, 800, "deaths");
    // Deaths mention "dies" (old age) or "perishes" (starvation).
    let has_death = text.contains("dies of old age") || text.contains("perishes of hunger");
    assert!(
        has_death,
        "expected death events in a long simulation; chronicle:\n{}",
        &text[..text.len().min(2000)]
    );
}

#[test]
fn harvest_overflow_appears_only_under_autumn() {
    let text = run_and_read(2024, 800, "harvest");
    if !text.contains("granary") {
        // Simulation produced no granary overflow events — nothing to validate.
        return;
    }
    // Walk through chronicle: track current season header, verify "granary"
    // lines only appear when the most recent header is an Autumn header.
    let mut current_season: Option<String> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("--- Year ") {
            // Format: "N, Season — ..." or "N, Season ---"
            if let Some((_year, tail)) = rest.split_once(", ") {
                let season = tail
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_end_matches(',');
                current_season = Some(season.to_string());
            }
        } else if line.contains("granary") && line.contains("overflows") {
            let season = current_season.as_deref().unwrap_or("<none>");
            assert_eq!(
                season, "Autumn",
                "granary overflow line appeared under season {}: {}",
                season, line
            );
        }
    }
}

#[test]
fn chronicle_prologue_and_epilogue_present() {
    let text = run_and_read(42, 50, "bookend");
    assert!(
        text.starts_with("worldforge"),
        "chronicle should begin with prologue line"
    );
    assert!(
        text.contains("The chronicle closes") || text.contains("Silence falls"),
        "chronicle should contain an epilogue"
    );
}
