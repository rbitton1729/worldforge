# worldforge

A world simulation engine. Simple rules, emergent civilizations, and a narrative that writes itself.

> Not a game. Not a dashboard. A world that lives.

## What is this?

You define a world — geography, resources, basic needs of its inhabitants. Then press play. Thousands of agents follow simple rules, and from that emerges: trade routes, wars, cultural shifts, alliances, famines, golden ages. A chronicle records everything, readable like a history book.

## Features

- **Hex grid world** with layered noise terrain generation (elevation → temperature → moisture → biomes)
- **Agents** with hunger, foraging, movement, reproduction, and death from old age
- **Settlements** that form when agents cluster, with stockpiles and population dynamics
- **Economy** — merchants travel between settlements, granaries fill at harvest
- **Conflict** — warriors raid neighbors, blood feuds form, alliances are pledged, settlements are sacked
- **Culture** — settlements develop traits (mercantile, militant) from their behavior, legendary events are recorded
- **Chronicle** — everything is narrated like a history book
- **Deterministic** — seeded RNG for reproducible runs

## Example Output

```
worldforge — seed 42 — 80×40 world — 300 souls

--- Year 1, Spring ---
A band of 7 settlers gathers upon the plains. They name the place Stenhold.
A band of 8 settlers gathers a day's walk from Stenhold. They name the place Wynstow.
*** The Battle of Ashvale — 3 warriors fall ***

--- Year 2, Summer — 369 souls across 24 settlements ---
3 souls depart the starving halls of Stonemoor.
A merchant arrives at Wynfall bearing grain from distant Ashford.

--- Year 4, Spring ---
Ashford becomes known as a haven of trade.
Stenhold is put to the torch. The smoke rises above empty fields.

The chronicle closes. 124 souls endure across 19 settlements.
```

## Quick Start

```bash
# Run with defaults (200 agents, 80x40 map)
cargo run

# Reproducible run with a seed
cargo run -- --seed 42 --ticks 500

# Larger world, more agents
cargo run -- --seed 42 --agents 400 --width 120 --height 60 --ticks 1000

# Faster (no delay between ticks)
cargo run -- --seed 42 --rate 0 --ticks 500

# Save chronicle to file
cargo run -- --seed 42 --chronicle history.txt --ticks 1000
```

## CLI

```
OPTIONS:
    -n, --agents <N>        Initial population [default: 200]
    -s, --seed <SEED>       RNG seed for reproducibility
    -r, --rate <TICKS/SEC>  Simulation speed [default: 1]
    -t, --ticks <N>         Total ticks to simulate (0 = infinite) [default: 0]
    -c, --chronicle <FILE>  Write chronicle to file (default: stdout)
    -w, --width <N>         Map width [default: 80]
    -h, --height <N>        Map height [default: 40]
        --help              Print help
```

## See Also

- [DESIGN.md](DESIGN.md) — full design document with systems, architecture, and phases
