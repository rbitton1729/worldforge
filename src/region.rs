use crate::world::{Biome, Tile};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::collections::{HashSet, VecDeque};

/// A named swath of land — a flood-filled cluster of connected same-family biomes.
#[derive(Debug, Clone)]
pub struct Region {
    pub name: String,
    pub center: (i32, i32),
    pub biome: Biome,
    pub tile_count: usize,
}

/// Clusters smaller than this are unremarkable and stay unnamed.
const MIN_REGION_SIZE: usize = 20;
/// Clusters larger than this are split into named sub-regions.
const MAX_REGION_SIZE: usize = 120;

/// Biome families for region grouping — plains and forest merge, deserts and
/// tundra remain separate, ocean is never part of a region.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Group {
    Lowland,
    Hills,
    Mountains,
    Desert,
    Tundra,
    Coast,
}

fn biome_group(b: Biome) -> Option<Group> {
    match b {
        Biome::Ocean => None,
        Biome::Plains | Biome::Forest => Some(Group::Lowland),
        Biome::Hills => Some(Group::Hills),
        Biome::Mountains => Some(Group::Mountains),
        Biome::Desert => Some(Group::Desert),
        Biome::Tundra => Some(Group::Tundra),
        Biome::Coast => Some(Group::Coast),
    }
}

fn names_for(biome: Biome) -> &'static [&'static str] {
    match biome {
        Biome::Plains => &[
            "the Green Vale",
            "the Golden Reach",
            "Sunfield",
            "the Broadmead",
            "the Open Country",
            "the Midlands",
            "the Longmere",
        ],
        Biome::Forest => &[
            "the Deepwood",
            "the Greenmere",
            "Thornwood",
            "the Elder Grove",
            "the Dark Canopy",
            "the Whispering Wood",
            "the Hollowoods",
        ],
        Biome::Hills => &[
            "the Shattered Hills",
            "the Stoneroll",
            "the Windreaches",
            "the Barrow Hills",
            "the Greylands",
            "the Highland Marches",
        ],
        Biome::Mountains => &[
            "the Spine of the World",
            "the Grey Peaks",
            "the Ironbound",
            "the Frostcrown",
            "the Cragmarch",
            "the Teeth",
        ],
        Biome::Desert => &[
            "the Ashlands",
            "the Dustwaste",
            "the Sunscorch",
            "the Sandveil",
            "the Embers",
            "the Scorched Reach",
        ],
        Biome::Tundra => &[
            "the Frozen Wastes",
            "the Pale Reach",
            "the Icebound",
            "the Coldmere",
            "the Silent North",
            "the Last White",
        ],
        Biome::Coast => &[
            "the Saltwind Coast",
            "the Tidemark",
            "the Seawall",
            "the Brinelands",
            "the Foam Shore",
            "the Shores of Thorn",
            "the Shores of Wyn",
            "the Shores of Mara",
            "the Shores of Kelvale",
        ],
        // Ocean shouldn't get named, but return something coherent if asked.
        Biome::Ocean => &["the Open Sea"],
    }
}

fn hex_neighbors(col: i32, row: i32) -> [(i32, i32); 6] {
    let odd = row & 1 == 1;
    if odd {
        [
            (col, row - 1),
            (col + 1, row - 1),
            (col - 1, row),
            (col + 1, row),
            (col, row + 1),
            (col + 1, row + 1),
        ]
    } else {
        [
            (col - 1, row - 1),
            (col, row - 1),
            (col - 1, row),
            (col + 1, row),
            (col - 1, row + 1),
            (col, row + 1),
        ]
    }
}

fn tile_idx(col: i32, row: i32, width: u32, height: u32) -> Option<usize> {
    if col < 0 || row < 0 || col as u32 >= width || row as u32 >= height {
        return None;
    }
    Some(row as usize * width as usize + col as usize)
}

/// Flood-fill the tile grid into named regions. Returns the regions in order
/// plus a per-tile lookup (`region_of[tile_idx]`) into that vec.
pub fn detect_regions(
    tiles: &[Tile],
    width: u32,
    height: u32,
    seed: u64,
) -> (Vec<Region>, Vec<Option<u16>>) {
    let total = (width * height) as usize;

    // First pass: group contiguous same-family tiles into raw clusters.
    let mut cluster_id: Vec<Option<u32>> = vec![None; total];
    let mut clusters: Vec<Vec<usize>> = Vec::new();

    for start in 0..total {
        if cluster_id[start].is_some() {
            continue;
        }
        let group = match biome_group(tiles[start].biome) {
            Some(g) => g,
            None => continue,
        };
        let cid = clusters.len() as u32;
        let mut members: Vec<usize> = Vec::new();
        let mut queue: VecDeque<usize> = VecDeque::new();
        queue.push_back(start);
        cluster_id[start] = Some(cid);
        while let Some(i) = queue.pop_front() {
            members.push(i);
            let c = (i % width as usize) as i32;
            let r = (i / width as usize) as i32;
            for (nc, nr) in hex_neighbors(c, r) {
                if let Some(ni) = tile_idx(nc, nr, width, height) {
                    if cluster_id[ni].is_none()
                        && biome_group(tiles[ni].biome) == Some(group)
                    {
                        cluster_id[ni] = Some(cid);
                        queue.push_back(ni);
                    }
                }
            }
        }
        clusters.push(members);
    }

    // Second pass: drop tiny clusters, split oversize ones, name the survivors.
    let mut rng = ChaCha8Rng::seed_from_u64(seed.wrapping_add(0xBEEF));
    let mut region_of: Vec<Option<u16>> = vec![None; total];
    let mut regions: Vec<Region> = Vec::new();
    let mut used: HashSet<String> = HashSet::new();

    for members in clusters {
        if members.len() < MIN_REGION_SIZE {
            continue;
        }
        let chunks = split_cluster(&members, width, height, MAX_REGION_SIZE);
        for chunk in chunks {
            if chunk.len() < MIN_REGION_SIZE {
                continue;
            }
            let dominant = dominant_biome(&chunk, tiles);
            let center = centroid(&chunk, width);
            let name = pick_name(dominant, &mut rng, &mut used, regions.len());
            let ridx = regions.len() as u16;
            for &m in &chunk {
                region_of[m] = Some(ridx);
            }
            regions.push(Region {
                name,
                center,
                biome: dominant,
                tile_count: chunk.len(),
            });
        }
    }

    (regions, region_of)
}

/// Split a too-large cluster into roughly-balanced sub-clusters by seeded BFS.
fn split_cluster(
    members: &[usize],
    width: u32,
    height: u32,
    cap: usize,
) -> Vec<Vec<usize>> {
    let n_chunks = (members.len() + cap - 1) / cap;
    if n_chunks <= 1 {
        return vec![members.to_vec()];
    }
    let target = (members.len() + n_chunks - 1) / n_chunks;
    let member_set: HashSet<usize> = members.iter().copied().collect();
    let mut assigned: HashSet<usize> = HashSet::new();
    let mut chunks: Vec<Vec<usize>> = Vec::new();
    for &seed in members {
        if assigned.contains(&seed) {
            continue;
        }
        let mut chunk: Vec<usize> = Vec::new();
        let mut queue: VecDeque<usize> = VecDeque::new();
        queue.push_back(seed);
        assigned.insert(seed);
        while let Some(i) = queue.pop_front() {
            if chunk.len() >= target {
                break;
            }
            chunk.push(i);
            let c = (i % width as usize) as i32;
            let r = (i / width as usize) as i32;
            for (nc, nr) in hex_neighbors(c, r) {
                if let Some(ni) = tile_idx(nc, nr, width, height) {
                    if member_set.contains(&ni) && !assigned.contains(&ni) {
                        assigned.insert(ni);
                        queue.push_back(ni);
                    }
                }
            }
        }
        chunks.push(chunk);
    }
    chunks
}

fn dominant_biome(chunk: &[usize], tiles: &[Tile]) -> Biome {
    let mut counts: [(Biome, u32); 8] = [
        (Biome::Ocean, 0),
        (Biome::Coast, 0),
        (Biome::Plains, 0),
        (Biome::Forest, 0),
        (Biome::Hills, 0),
        (Biome::Mountains, 0),
        (Biome::Desert, 0),
        (Biome::Tundra, 0),
    ];
    for &m in chunk {
        let b = tiles[m].biome;
        for entry in counts.iter_mut() {
            if entry.0 == b {
                entry.1 += 1;
                break;
            }
        }
    }
    counts.iter().max_by_key(|e| e.1).unwrap().0
}

fn centroid(chunk: &[usize], width: u32) -> (i32, i32) {
    let mut sum_c: i64 = 0;
    let mut sum_r: i64 = 0;
    for &m in chunk {
        sum_c += (m % width as usize) as i64;
        sum_r += (m / width as usize) as i64;
    }
    let n = chunk.len() as i64;
    ((sum_c / n) as i32, (sum_r / n) as i32)
}

fn pick_name(
    biome: Biome,
    rng: &mut ChaCha8Rng,
    used: &mut HashSet<String>,
    region_seq: usize,
) -> String {
    let pool = names_for(biome);
    for _ in 0..20 {
        let candidate = pool[rng.gen_range(0..pool.len())].to_string();
        if !used.contains(&candidate) {
            used.insert(candidate.clone());
            return candidate;
        }
    }
    // Pool is exhausted — disambiguate by appending a numeral.
    let base = pool[rng.gen_range(0..pool.len())];
    let name = format!("{} {}", base, roman_numeral(region_seq + 1));
    used.insert(name.clone());
    name
}

fn roman_numeral(n: usize) -> String {
    // Small helper so the suffix reads as "the Deepwood II" instead of "the Deepwood 2".
    const TABLE: &[(usize, &str)] = &[
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut n = n;
    let mut out = String::new();
    for &(v, s) in TABLE {
        while n >= v {
            out.push_str(s);
            n -= v;
        }
    }
    out
}
