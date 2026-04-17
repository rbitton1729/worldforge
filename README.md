# worldforge

A world simulation engine. Simple rules, emergent civilizations, and a narrative that writes itself.

> Not a game. Not a dashboard. A world that lives.

## What is this?

You define a world — geography, resources, basic needs of its inhabitants. Then press play. Thousands of agents follow simple rules, and from that emerges: trade routes, wars, cultural shifts, alliances, famines, golden ages. A chronicle records everything, readable like a history book.

## Quick Start

```bash
# Run with defaults (200 agents, 80x40 map)
cargo run

# Reproducible run with a seed
cargo run -- --seed 42

# Faster simulation
cargo run -- --rate 10

# With TUI
cargo run -- --gui
```

## See Also

- [DESIGN.md](DESIGN.md) — full design document with systems, architecture, and phases
