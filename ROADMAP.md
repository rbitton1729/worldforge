# worldforge — Roadmap

## v0.1.0 — Foundation ✅
The world lives. Agents eat, starve, reproduce, die of old age.
Settlements form, trade, raid each other, develop traits. The chronicle narrates.

## v0.2.0 — Geography ✅
- [x] Rivers (follow elevation to coast, settlements near rivers get food bonus)
- [x] Named regions — not just settlements, but the land itself gets names
  ("the Ashlands", "the Velmara Expanse") based on biome clusters
- [x] Terrain depletion — forests shrink when over-harvested, fertility drops with overfarming
- [x] Climate shifts on long timescales (slow temperature drift, not dramatic events)
- [x] Passes through mountains — make mountains traversable but costly

## v0.3.0 — Culture
- [x] Great individuals — agents that do exceptional things get named and remembered
  (epithets like "the Conqueror", "the Wanderer" based on deeds: raiding, trading, surviving)
- [x] Customs emerge — settlements develop traditions from behavior patterns
  ("The people of Thornhold feast at harvest", "Velmara sends its youth on a year's journey")
- [x] Languages — settlements that are far apart develop different name generators
- [x] Religion — emerges from attempts to explain famine/abundance
  ("The people of Calwold begin to worship the river")

## v0.4.0 — Conflict Deepens
- [x] Alliances matter — allied settlements share warriors during raids
- [ ] Territory claims — settlements mark borders, crossing them provokes conflict
- [ ] Sieges — not just raids, but sustained blockades of starving settlements
- [x] Conquest — victorious settlement absorbs the loser's population
- [x] War chronicles — multi-season wars with named battles, retreats, last stands

## v0.5.0 — Economy Deepens
- [ ] Trade routes form and persist (merchants prefer established paths)
- [ ] Specialization — settlements near mountains become miners, coast becomes fishers
- [ ] Currency emerges (not designed, but units of value appear from trade patterns)
- [ ] Economic pressure — trade embargo between feuding settlements
- [ ] Famine relief — wealthy settlements send food to starving allies

## v0.6.0 — Emergent Technology
- [ ] No tech tree. Instead: skills compound through use.
  Settlement that farms a lot → discovers better farming (yields increase)
  Settlement that fights a lot → develops better tactics (raid advantage)
  Settlement that trades a lot → develops logistics (merchants travel faster)
- [ ] Knowledge spreads — techniques propagate between settlements via merchants
- [ ] Innovation is rare and spread unevenly, creating inequality and conflict

## v0.7.0 — Natural Events
- [ ] Droughts — multi-season food reduction in a region
- [ ] Floods — rivers overflow, damage settlements, enrich farmland
- [ ] Plagues — population reduction, spread between connected settlements
- [ ] Good harvests — bountiful years, population booms
- [ ] All events are rare, regional, and create narrative pressure (not random disasters)

## v0.8.0 — TUI
- [ ] Hex map visualization with terrain colors
- [ ] Real-time population/resource graphs
- [ ] Chronicle tail (latest entries scrolling)
- [ ] Interactive controls (pause, speed up, zoom, inspect settlement)
- [ ] Minimap and territory view

## v0.9.0 — Scale
- [ ] Multi-threaded simulation (agent updates in parallel)
- [ ] Save/load world state
- [ ] Run across sessions (pause and resume a simulation)
- [ ] Larger maps (200x100, 1000+ agents)
- [ ] Export chronicle as formatted text/HTML

## v1.0.0 — The Living World
- [ ] Everything above working together
- [ ] A simulation that produces genuinely compelling emergent narratives
- [ ] You can run it, come back in an hour, and read a history that surprises you
- [ ] Performance that handles 1000+ agents at interactive speeds

---

## Design Constraints (never violate)

1. **Define pressures, not outcomes.** No scripted events, no predetermined narrative.
2. **Emergence over design.** If it needs a special case, the rules are wrong.
3. **The chronicle is the product.** Everything exists to produce a good story.
4. **Deterministic.** Same seed = same history. Always.
5. **Simple rules, complex behavior.** If an agent has more than ~5 behavioral rules, something is wrong.
