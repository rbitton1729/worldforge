# worldforge

A world simulation engine. Simple rules, emergent civilizations, and a narrative that writes itself.

## Vision

You define a world — its geography, resources, and the basic needs of its inhabitants. Then you press play. Thousands of agents follow simple rules, and from that emerges: trade routes, wars, cultural shifts, alliances, famines, golden ages. A chronicle records everything that happens, readable like a history book.

Not a game. Not a dashboard. A world that lives.

## Core Design Principles

- **Simple rules, complex outcomes.** Every agent has a small set of needs and behaviors. Complexity is emergent, not designed.
- **No predetermined narrative.** We don't script events. We create conditions for events to arise.
- **Readable output.** The primary output is a chronicle — a text narrative of what happened. Secondary: optional TUI to watch it unfold live.
- **Deterministic option.** Support seeded RNG so the same world + same rules = same history. Makes it interesting to compare "what if" scenarios.

## Systems

### 1. Geography

The world is a 2D hex grid. Each tile has:

- **Terrain type:** ocean, coast, plains, forest, hills, mountains, desert, tundra
- **Resources:** food (varies by terrain), water, wood, stone, ore, fertility
- **Climate:** temperature band, rainfall — affects food production
- **Biome:** derived from terrain + climate, affects what can grow/hunt

World generation uses layered noise (elevation → temperature → moisture) to produce coherent biomes. Coastlines, river valleys, mountain ranges — all emergent from the noise, not placed by hand.

### 2. Agents

Agents are the inhabitants. They start as simple villagers and can specialize over time.

**Needs (drives all behavior):**
- **Hunger** — needs food, depletes over time
- **Safety** — needs shelter, threatened by danger
- **Social** — proximity to other agents improves wellbeing

**Attributes:**
- Health (0-100)
- Age (ticks upward, eventually death)
- Skill bias (slight predisposition toward farming, fighting, crafting)
- Loyalty (which settlement they belong to)

**Behaviors (priority-ordered):**
1. If starving → seek food (forage, trade, or steal)
2. If threatened → flee or fight
3. If idle → gather resources, build, or socialize
4. Random drift between behaviors for variety

**Emergent roles** (not assigned, just what they end up doing):
- Farmer: stays near fertile land, produces food surplus
- Warrior: patrols, defends, raids
- Merchant: moves between settlements, trades surplus
- Builder: constructs structures

### 3. Settlements

When enough agents cluster in one place, a settlement forms. Settlements have:

- **Population** — count of loyal agents
- **Stockpile** — shared resource pool
- **Structures** — built over time (granary, walls, market)
- **Territory** — claimed tiles around the settlement

Settlements can grow, decline, or be destroyed. A settlement with no food and no population is abandoned.

### 4. Economy

Simple supply/demand:
- Each settlement produces surplus of whatever its terrain offers
- Merchants (agents who travel) carry goods between settlements
- Trade happens when a merchant arrives at a settlement with goods it lacks
- Price isn't explicit — it's just "I have wood, you have food, let's swap"
- Trade routes emerge from repeated merchant paths

### 5. Conflict

- Warriors can raid other settlements for resources
- Raid success depends on attacker vs defender strength (warrior count + structures)
- Successful raids steal resources, may kill defenders
- Failed raids result in attacker casualties
- Repeated raids → enmity → wars
- Alliances can form between settlements that trade frequently

### 6. Culture (later phase)

- Settlements develop traits over time: militaristic, mercantile, isolationist
- Traits emerge from behavior patterns, not assigned
- Naming: settlements and notable agents get generated names
- "Legends": exceptional events (big wars, famines, great trades) get highlighted in the chronicle

## Chronicle (Primary Output)

The chronicle is a running text narrative. Examples:

```
--- Year 1, Spring ---
The world awakens. 200 souls draw breath on the plains of Verada.

--- Year 1, Summer ---
A band of 12 settlers has gathered near the river Elda.
They name the place Thornhold.

--- Year 3, Winter ---
Famine grips Thornhold. The granary is empty.
Fourteen souls perish before the spring thaw.

--- Year 5, Autumn ---
A caravan from Duskmoor arrives at Thornhold bearing grain.
The two settlements have found common cause.

--- Year 8, Spring ---
Warriors of Velmara sack the outlying farms of Thornhold.
The defenders are overwhelmed. Thornhold loses half its stores.
```

The chronicle is written to stdout (or a file). It's the main way you experience the simulation.

## TUI (Secondary, Optional)

A simple ratatui interface showing:
- The hex map with terrain colors and settlement markers
- A population/resource graph over time
- The chronicle tail (latest entries)

Toggle with a flag. Default is just the chronicle scrolling past in the terminal.

## Architecture

```
src/
├── main.rs          — CLI args, simulation loop, chronicle output
├── world.rs         — World struct, tile grid, geography generation
├── agent.rs         — Agent struct, behavior tree, movement
├── settlement.rs    — Settlement logic, stockpiles, territory
├── economy.rs       — Trade, supply/demand, merchant behavior
├── conflict.rs      — Raid logic, combat, alliances
├── chronicle.rs     — Narrative generation from events
├── gen.rs           — World generation (noise, biomes, placement)
└── tui.rs           — Optional TUI (behind feature flag)
```

**Dependencies:**
- `noise` — terrain generation
- `rand` + `rand_chacha` — deterministic RNG
- `ratatui` + `crossterm` — TUI (optional)
- No async, no networking. Pure simulation loop.

## CLI

```
worldforge [OPTIONS]

OPTIONS:
    -n, --agents <N>        Initial population [default: 200]
    -s, --seed <SEED>       RNG seed for reproducibility
    -r, --rate <TICKS/SEC>  Simulation speed [default: 1]
    -t, --ticks <N>         Total ticks to simulate (0 = infinite) [default: 0]
    -c, --chronicle <FILE>  Write chronicle to file (default: stdout)
    -g, --gui               Enable TUI mode
    -w, --width <N>         Map width [default: 80]
    -h, --height <N>        Map height [default: 40]
        --help              Print help
```

## Phases

### Phase 1: Foundation
- Hex grid, terrain generation, biome assignment
- Agents with basic needs (hunger, movement)
- Simple foraging — agents eat, or die
- Chronicle output

### Phase 2: Settlements
- Agents cluster and form settlements
- Resource stockpiling
- Basic structures (granary)
- Population dynamics (birth, death, migration)

### Phase 3: Economy
- Merchants emerge
- Trade between settlements
- Supply/demand influences movement
- Trade routes

### Phase 4: Conflict
- Warriors, raids, combat
- Alliances and enmity
- Territory disputes
- Sacking and conquest

### Phase 5: Culture
- Settlement traits
- Naming generation
- Legend highlighting
- "Historical" narrative arcs

### Phase 6: TUI
- Map visualization
- Real-time stats
- Interactive controls (pause, speed up, zoom)

## Evolution Philosophy

**The cardinal rule: define the pressures, not the outcomes.**

There is no tech tree. No advancement stages. No predetermined path from village to empire. Evolution is entirely emergent from agent behavior and environmental pressure.

### Population as the primary driver

- Agents reproduce when well-fed and near others
- Population grows → needs more food → needs more territory
- Population overshoots carrying capacity → famine → die-off → recovery
- This alone creates boom/bust cycles without any scripted events

### Settlements evolve through what they build, not what they "unlock"

- A settlement with food surplus attracts more agents
- More agents → more labor → someone builds a granary
- Granary → survives longer winters → grows bigger
- Eventually walls (if raided), markets (if merchants visit)
- No one decides "now we build walls." It happens because conditions demand it.

### Agent specialization is behavioral, not assigned

- An agent that keeps finding food near fertile land → stays there → becomes a "farmer"
- An agent that keeps fighting → gets better at it → becomes a "warrior"
- Skills compound through repetition, like muscle memory
- Roles emerge from geography and circumstance, not from a class picker

### Resources create territory pressure

- Fertile land is finite. Good spots get claimed.
- Settlements with good land thrive. Settlements on poor land struggle.
- Struggling settlements either innovate (trade), migrate, or die.
- Conflict happens at the edges where territories meet over contested resources.

### The world is mostly static — agents reshape it

- Terrain doesn't change (mostly). Forests can be depleted by over-harvesting.
- But the *meaning* of a tile changes: a forest near a growing settlement becomes farmland through use
- Climate shifts on very long timescales — not in a gamey "ice age event" way, just slow drift

### Emergent "ages" (not designed)

Instead of "Bronze Age → Iron Age," you see patterns like:

1. **Early:** scattered foragers, no settlements
2. **Settlement:** clusters form, basic farming
3. **Expansion:** settlements grow, territories bump into each other
4. **Conflict:** raids over resources, alliances form
5. **Trade:** surviving settlements specialize and exchange
6. **Collapse:** overextension, famine, wars drain populations
7. **Recovery:** new settlements form in the aftermath

These "ages" aren't programmed — they're what naturally happens with limited resources and agents that need things. The goal is to create conditions for narrative to emerge.

### What is deliberately excluded

- No explicit technology progression
- No civilization "levels" or advancement stages
- No predetermined path from village to empire
- No events that fire at specific times
- No script-driven narrative beats

### The risk

The simulation might produce chaos instead of narrative — random death spirals with no interesting patterns. If that happens, we tune the rules, not script events. The philosophy is: **define the pressures, not the outcomes.**

## Open Questions

- How fast should time pass? Real-time vs tick-based with user control?
- Should agents have individual names from the start, or only "notable" ones?
- How detailed should combat be? (Simple ratio check vs multi-tick battles?)
- Should the world be finite (bounded grid) or wrap around (toroidal)?
- Multi-threaded simulation for larger worlds?
