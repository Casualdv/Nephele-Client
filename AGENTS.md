# AGENTS.md — Nephele Client

Read this before touching anything. It is the durable memory of the project: every
session starts here, and it is the only file that should be updated by default as part
of a task. Keep it accurate when you change behaviour.

---

## 0. Standing rules (every session, no exceptions)

1. **Work in your own worktree.** Create or use `np-<lane>` on branch `<lane>`; §12
   lists the lanes. **Never edit the main checkout** at
   `C:\Users\dariu\Desktop\Nephele Client` — that is where merges land and where every
   other lane's rebase passes through. Build into your own `target/`.
2. **One client at a time.** Test through `scripts/run_client.ps1 -Session <lane>`,
   never a bare `nephele.exe`. Two clients fight over the window and over a single
   Microsoft account (which is how a 503 from the session server happens).
3. **Stay in your lane's files.** If you need a change in another lane — a protocol
   field, an atlas change — say so in your report instead of editing it. A second
   copy of the atlas or of a parser is worse than waiting.
4. **Licence.** `Nephele-private` is read-only reference: read the decompiled 1.8.9
   sources, reimplement in Rust, cite the class in a comment. Never copy code, assets
   or data into this repository. GPL projects (`PommeMC`, `minecrab`) are read-only.
5. **Gates before pushing.** `cargo fmt --all -- --check`,
   `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace`, `scripts/check_public_tree.ps1`. Push your branch; **do
   not merge to `main`** — the integrator merges.
6. **Report honestly.** What changed, what you verified and with which command or
   screenshot, and what you did **not** verify. Write "not verified" rather than
   implying it. A known gap is worth more than a confident guess.

---

## 1. What this is

Nephele is a **from-scratch Rust client for Minecraft Java Edition 1.8.9**
(protocol 47) — the "1.8 PvP" era. The target product is a legitimate
Lunar/Feather-style client: fast, low-GPU, works on weak laptops, PvP-first.

Two goals pull against each other and both matter:

1. **Fidelity.** Terrain, lighting and physics should be indistinguishable from
   vanilla, because PvP players notice block-level differences.
2. **Performance.** The renderer and worldgen must run on integrated graphics.

Repo: `C:\Users\dariu\Desktop\Nephele Client`
Remote: `https://github.com/Casualdv/Nephele-Client` (branch `main`)

---

## 2. Compliance — read this before you write code

This is the single most important constraint in the project.

- **The repo ships zero Mojang code, assets, or data.** No textures, no sound, no
  jars, no extracted class files, no copied resource tables.
- `nephele-prepare` builds resources **on the user's own machine** from the user's own
  `1.8.9.jar`, verified by SHA-1 against `research-lock.json`. Derived output goes to
  `.local/` which is gitignored.
- `scripts/check_public_tree.ps1` is the publish gate and runs in CI. If it fails,
  the tree is not publishable.
- A non-affiliation disclaimer is required in the README.

**Licence rules for reference code:**

| Source | Licence | Rule |
|---|---|---|
| `Lea-fish/Leafish` | MIT / Apache-2.0 | May copy with attribution in `NOTICE` |
| `azalea-rs/azalea` | MIT | May copy with attribution |
| `Oery/origami` | MIT | May copy with attribution |
| `horizon-rust/horizon` | MIT | **Pattern only.** A from-scratch 1.26-era reimplementation with ~281 differential harnesses (`tests/`, `tools/lab.py`) diffed against vanilla. Its corpus/oracle methodology is the same one we should use; its protocol and packet ids are a different version, so none of it runs against 1.8.9 unchanged. |
| `Thinkofname/steven`, `iceiix/stevenarella` | permissive | May copy with attribution |
| `PommeMC/Client`, `minecrab` | **GPL-3.0** | **Never copy.** Read-only reference |
| `Exopteron/opengl-classic-client` | none stated | Off limits |

None of the reference clients generate terrain, and none of the MIT ones had working
auth — so worldgen and auth are genuinely ours.

I am not a lawyer and this is not legal advice. Get a real review before a public or
commercial release.

---

## 3. Environment — every session, before any cargo/java call

This machine's `vcvars64.bat` sets nothing (System Restore damage). A working env
script lives outside the repo; source it first, every call:

```powershell
Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass -Force
. "C:\Users\dariu\AppData\Local\Temp\opencode\cargo-env.ps1"
```

**Permanent fix already applied at User scope** (new terminals pick it up):

- `LIB` = MSVC `VC\Tools\MSVC\14.44.35207\lib\x64` + SDK `10.0.26100.0\{ucrt,um}\x64`
- `INCLUDE` = MSVC include + SDK `{ucrt,shared,um}`
- `PATH` += MSVC `bin\HostX64\x64` — needed because `ring` shells out to `cl.exe`

If a build suddenly fails on cl.exe or missing libs, this is the first thing to check.

**Java toolchain** for the parity oracle: Temurin JDK 8u504 at
`C:\Users\dariu\Desktop\Nephele-private\tools\jdk8`.

---

## 4. Commands

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --bins
.\scripts\check_public_tree.ps1          # publish gate

cargo run -p nephele -- view --seed 12345        # offline world demo
cargo run -p nephele -- play <host:port>         # offline-mode join
cargo run -p nephele -- play <host:port> --online  # Microsoft-authed join
cargo run -p nephele -- auth [--legacy] [--client-id <id>]
cargo run -p nephele-launcher                    # the launcher UI

# development shortcuts (never the only way to reach a screen):
$env:NEPHELE_SCREEN="servers"   # title|servers|connect|settings|creative|pause|local
$env:NEPHELE_DATA_DIR="..."     # override the user data directory
```

CI runs fmt, clippy with `-D warnings`, tests, and the publish gate. **A commit that
breaks clippy breaks CI** — `cargo clippy` must be run before every commit, and note
that chaining with `;` in PowerShell does **not** stop on failure.

---

## 5. Repo map

```
crates/
  nephele/              CLI binary: view / play / auth / prepare flows
  nephele-launcher/     winit + UiBatch launcher; spawns `nephele play` as a child
  nephele-protocol/     packets, codecs, crypto (server_id_hash, rsa_encrypt)
  nephele-client/       connection lifecycle, mining, physics
  nephele-auth/         device-code OAuth → XBL → XSTS → Minecraft
  nephele-world/        worldgen + chunk data (the parity-critical crate)
  nephele-render/       mesher, chunk mesh, shader
  nephele-assets/       texture array, atlas, resource paths
  nephele-cli/src/      lib code used by the CLI binary:
      live.rs           network + local game loops, screens, input
      viewer.rs         offline world viewer
      ui.rs             immediate-mode UI batch + screens (draw_*)
      commands.rs       client-side chat commands (/gamemode, /setblock, ...)
      paths.rs          per-user data directory + legacy `.local/` migration
      mesher.rs         CLI-side mesh helpers
      servers.rs        saved server list: entries, ordering, persistence, ping board
      settings.rs       persisted options
      skins.rs          skin fetch/cache
```

### User files (`.local/` is legacy)

`paths::user_data_dir()` is the single answer to "where do user files go":
`NEPHELE_DATA_DIR` → `%APPDATA%\Nephele` → `$XDG_DATA_HOME/nephele` →
`~/.local/share/nephele`. **`auth.json` and `servers.json` both live there** and
nothing writes anywhere else. A game directory is often read-only and is
per-install, so `.local/` was the wrong home for a token; `migrate_legacy_state`
(`main.rs`, Play command only) runs `paths::migrate_legacy_file` once per file,
which **copies** (never moves, never overwrites) a legacy `.local/` file when the
new one is absent. Leave the legacy file alone — it is the fallback. `.local/`
still owns the *prepared resources*, which are per-install by design.

### The multiplayer screen (`Screen::Servers`)

`Action::Multiplayer` opens it; `ui::draw_servers` draws it. Four columns per
row: **name+address, MOTD, version, ping**. `servers.rs` owns the data
(`ServerList`, `PingBoard`, `Ping`, `Status`), `live.rs` owns the fetching:

- Pings are asynchronous, one short-lived thread per address, each with its own
  current-thread tokio runtime and `nephele_protocol::ping(.., PING_TIMEOUT = 1s)`.
  Results return over an mpsc channel drained in `frame()`; the render thread
  never blocks. **There is no polling loop** — a fetch happens when the screen
  opens and when Refresh is pressed, and `PingBoard::begin` refuses to duplicate
  an already in-flight address.
- **Single click selects; it does not join.** Joining is the Join Server button,
  Enter, or a double-click (two clicks on the same address within
  `DOUBLE_CLICK = 400 ms`). This is deliberate: with per-row delete/favourite
  buttons, click-to-join was a misclick factory.
- Selection is stored as the **address string**, not a row index, because
  deleting an entry or favouriting one re-sorts the list. Rows still *act* by
  storage index (`ServerList::ordered_indices`) because `toggle_favourite` and
  `remove` take one.
- `Direct Connect` is a real button on this screen and the only UI route to
  `Screen::Connect`; its Escape/Cancel returns to the multiplayer screen.
  `NEPHELE_SCREEN=servers|connect|…` remains a development shortcut and must
  never be the *only* way to reach a screen.
- Note `sprites::BUTTON_DISABLED` **overlaps the hotbar-selection art** in this
  GUI atlas, so `draw_button_state` never draws it: a disabled button keeps the
  normal face and greys its label, exactly like vanilla 1.8.

`Future.md` at the repo root holds the product plan, milestone table, and open
decisions. Keep it in sync when direction changes; it is not auto-updated.

---

## 6. Worldgen — the heart of the project

Pipeline order per chunk (`VanillaGenerator` in `worldgen.rs`):

```
primer (base terrain density)
  → surface (biome surface blocks)
  → caves
  → ravines
  → ores
  → features (decoration)
```

**That single-pass model cannot reach block parity.** Vanilla splits it in two,
and the split is load-bearing (source-verified against MCP 1.8.9):

- **`provideChunk`**: base terrain → biome surface → caves → ravines →
  structure-start discovery (mineshaft, village, stronghold, scattered feature,
  ocean monument — layouts cached here) → chunk construction, biome storage,
  skylight. Surface and bedrock reseed from the chunk seed, so they do **not**
  depend on request order.
- **`populate`**: structure pieces → water lake → lava lake → 8 dungeon attempts
  → `BiomeDecorator` (**ores first**, then underwater sand/clay/gravel, trees,
  big mushrooms, flowers, grass, dead bushes, lilies, mushrooms, reeds, cacti,
  springs) → animal spawns → ice/snow.

Consequences we have not implemented:

- **Ore placement belongs to the population pass**, not `provideChunk`; it uses
  the population seed and `+8` offsets, so per-chunk ore generation can never
  match.
- Population is a **2×2 rule**: owner `(x,z)` runs only once `(x,z)`,
  `(x+1,z)`, `(x,z+1)`, `(x+1,z+1)` exist, and eligibility is checked in the
  order `(x,z)`, `(x-1,z)`, `(x,z-1)`, `(x-1,z-1)`. Writes cross the boundary.
- The decorating biome is sampled at `(chunkX*16+16, chunkZ*16+16)`.
- Population seed: two `nextLong()` draws, each converted `n/2*2+1`, then
  `(x*first + z*second) XOR seed`. Cave/ravine seeding uses a **17×17** source
  neighbourhood with **unforced** multipliers `(sx*A) XOR (sz*B) XOR seed`.
- Structure grids use `regionX*341873128712 + regionZ*132897987541 + seed + salt`
  (village 10387312, scattered 14357617, monument 10387313) with floor division
  for negative regions.


Environment: **overworld, sea level 63**, infinite generation, biome map from the
1.8 `GenLayer` stack.

### Parity status (measure against the reference world, never by eyeball)

| Stage | State | Number |
|---|---|---|
| Terrain height field | done | 96% of columns match |
| Surface blocks | done | 98.8% match (1.2% mismatch) |
| Caves | done | 308 air/chunk below y=56 |
| Ravines | done | 426 vs 435 reference (98%) |
| Ores | done, wrong stage | 5–12% above reference (caves absent when measured) |
| **Biome map** | **done** | **0 / 160,000 blocks differ** (seed 12345, reference server world), and 0 / 160,000 again after a full `column()` pass, read from `ChunkColumn::biomes()` |
| Features | partial | flowers, grass, dead bush, cactus, oak-shape trees |

**Every number above except the biome map was measured per chunk, which is a
different pipeline than vanilla's.** Treat them as "our implementation is close"
rather than parity until the population pass exists and they are re-measured on
a populated world.

**The oracle cannot settle this yet.** The second reference world now exists at
`Nephele-private/parity/server-random` (level `random`, seed 12345, DEFAULT,
empty generator settings, **structures on**, 625 chunks / 25×25 from spawn).
`parity/server` stays as-is for the structures-off comparison. Neither answers a
block-level question yet: the oracle emits surface heights, not block arrays, so
a 65,536-state per-chunk export is still the prerequisite. Compare only the
inner area (roughly ±8 chunks): the outermost ring lacks its populated
neighbours and is not expected to match.


### The biome bug — solved 2026-11-05

**Root cause: `GenLayerHills` was reseeding its variance layer.** Vanilla keeps
the hill-noise layer in a private field (`field_151628_d`), not in `parent`, so
`GenLayer.initWorldGenSeed` never reaches it: the two `GenLayerZoom`s above it
run with `worldGenSeed == 0` for the entire world. The Rust port called
`set_world_seed` on it, which changed the `(noise - 2) % 29 == 0/1` tests in
`GenLayerHills` (and therefore the hill/mutation choice) for roughly 1 cell in
29. It looks like an obvious bug to fix, and fixing it breaks parity. Pinned by
`hills_never_seeds_its_variance_layer`.

Measured on the structures-off reference world, 160,000 blocks (625 chunks):

| | Mismatched |
|---|---|
| Before | 11,492 (92.82% match) — 3→131 ×5606, 19→133 ×3022, 133→5 ×1946 |
| After | **0 (100%)** |

**Scale model** (verified again by the stage trace):

- `GenLayerBiome` → `BiomeEdge` → **Hills are 1:64**
- RareBiome → 4× Zoom (AddIsland at k=0, Shore at k=1) → Smooth → RiverMix are **1:4**
- Voronoi reads its parents at `(block - 2) >> 2`, output 1:1

`initChunkSeed` and `nextInt` are **verified byte-identical to vanilla** — four
multiply-add steps alternating x/z, and `nextInt` advancing with
`chunkSeed*M+A + worldGenSeed`. They were never the bug. Do not re-audit them.

**How it was found, and how to find the next one:** compare stages, not final
percentages. `dev.nephele.oracle.Trace` snapshots every layer inside one
continuous generation and `debug_stage_trace` replays the same stack in Rust
(see §7). The first differing stage was `hillNoiseZoom0` — 56 of 256 cells —
while its input `riverInit` matched exactly, which localised the fault to the
layer above the input rather than to any zoom arithmetic.

### Where the map reaches the screen — fixed 2026-11-08

A correct map is not a visible one. Two separate links were broken:

1. **Singleplayer never called this generator.** `live.rs::local_loop` streamed
   `TerrainGenerator` from `generation.rs` — the value-noise stand-in whose own
   module docs say it "is not vanilla's generator". Singleplayer now streams
   `VanillaGenerator`: the same code the parity harness measures.
2. **`VanillaGenerator::column` dropped the map.** It computed the 16×16 biome
   map, used it for the surface, cave and feature passes, then returned a
   column with `biomes: None`. Vanilla's `ChunkProviderGenerate` stores it as
   `Level.Biomes`, so `column()` now calls `set_biomes`. Pinned by
   `generated_chunks_carry_the_biome_map` and
   `generated_chunk_biomes_match_the_reference_world` — the latter reads the
   bytes back out of `ChunkColumn::biomes()` for all 625 reference chunks
   (160,000 cells, 0 mismatched), which is the same window as the GenLayer test
   but one layer closer to the screen.

Consumers of `ChunkColumn::biomes()`: `live.rs::biome_at` (sky/fog colour) on
`main`, and on the unmerged `render` branch `ColumnMeshJob.biomes` +
`atlas.tint` (grass, foliage, water, with the vanilla 3×3 neighbourhood
average). **On `main` there is still no per-block tint**: `nephele-assets`
bakes the plains colour into grass (`PLAINS_GRASS` / `PLAINS_FOLIAGE`), so grass
is plains-green everywhere until `render` merges. Every consumer falls back to
plains (1) when a column has no biomes, which is why the missing map read as
"all plains" rather than as an error. Server-joined worlds only needed the
render half: `decode.rs` already stores the S21 `Level.Biomes` payload.

---

## 7. The parity oracle

`C:\Users\dariu\Desktop\Nephele-private\oracle\` — a small Java harness compiled
against the real 1.8.9 classes, **never published**.

```powershell
cd C:\Users\dariu\Desktop\Nephele-private\oracle
.\build.ps1
..\tools\jdk8\bin\java.exe -cp out dev.nephele.oracle.Stages <stage> <seed> <x> <z> <w> <h>
..\tools\jdk8\bin\java.exe -cp out dev.nephele.oracle.Terrain <seed> <cx> <cz>
```

Stages: `mixed river riverSmooth riverInit hills hillsOnly hillNoise hillZoom1
riverChain biome biomeEdge base voronoi smoothed probe`.

**Caveat: `Stages` is only trustworthy at the final map.** Isolated stage
queries are corrupted by `IntCache` aliasing — Java's `hillsOnly` contradicts its
own `hillNoise` in the same window. I proved this; it invalidated a whole round
of "stages match" conclusions. The mechanism: `getIntCache` hands out fixed
256-int buffers for any request ≤ 256, and `resetIntCache` recycles them between
queries, so a stage query can read another query's leftovers. Never compare two
isolated `Stages` runs cell by cell.

### Stage trace (the trustworthy stage comparison)

`tools/Trace.java` + `trace.ps1` wrap every layer in a recorder and generate the
whole stack **once**, copying each layer's output immediately, so no buffer is
ever reused before it is read. It writes one file: magic `NEPHTRC1`, a record
count, then per record `name, areaX, areaZ, w, h, length, i32[length]`.

```powershell
cd C:\Users\dariu\Desktop\Nephele-private\oracle
.\trace.ps1                       # writes ..\parity\stage_trace.bin (81 stages)
cd C:\Users\dariu\Desktop\np-m1b
cargo test -p nephele-world --lib debug_stage_trace -- --ignored --nocapture
```

`debug_stage_trace` rebuilds the identical stack in Rust (`traced_stack`) and
prints one line per stage: window, mismatches, and the first differing cell. The
first `DIFF` is the answer — that is exactly how the Hills bug was found
(`riverInit` matched, `hillNoiseZoom0` did not, so the fault was the seed
propagation into the hill-noise branch, not any zoom arithmetic).

Default window covers the whole structures-off reference area. Re-run
`trace.ps1` after `build.ps1`, which deletes and regenerates `src/`.

### Reference world

Real 1.8.9 server, **seed 12345**, `generate-structures=false`, `online-mode=false`:

```
C:\Users\dariu\Desktop\Nephele-private\parity\server\world\region\
```

Server jar SHA-1 `b58b2ceb36e01bcd8dbf49c8fb66c55a9f0676cd`. This is the ground
truth for every parity number above.

### Pinned checksums (`research-lock.json`)

- client jar `1.8.9.jar` — sha1 `3870888a6c3d349d3771a3e9d16c9bf5e076b908`, 8,461,484 bytes
- asset index `1.8` — sha1 `f6ad102bca53b1a58358f16e376d548d44933ec`, 78,494 bytes
- local copy: `C:\Users\dariu\AppData\Roaming\.minecraft\versions\1.8.9\1.8.9.jar`

### Decompiled source

`C:\Users\dariu\Desktop\Nephele-private\references\mcp-919\` — the 1.8.9 source.
**Never publish, never copy into the repo.** Read it, reimplement it.

---

## 8. Auth

Flow: device code → Xbox Live → XSTS → Minecraft services → profile → join.

- Endpoints: `login.microsoftonline.com/consumers/oauth2/v2.0/{devicecode,token}`
- Scope: `XboxLive.signin offline_access`
- `login.live.com` **rejects newly registered Entra apps** ("does not exist in tenant
  'Microsoft Services'"). Switching to a legacy app id was my bug, not the endpoint's.
- `--legacy` toggles the old `login.live.com` + `service::user.auth.xboxlive.com::MBI_SSL`
  path. It currently uses a borrowed app id (`00000000441cc96b`, the Switch app) —
  **testing only, cannot ship**.

**Blocked:** the user has no usable Entra directory (M365 Dev Program rejected; the
university tenant `didadec.onmicrosoft.com` / `a6eb79fa-c4a9-4cce-818d-b85274d15305`
returns 401). Azure for Students (no card) or Azure free is the path. The code is
otherwise complete and works; only the shippable app id is missing.

`StoredSession` persists to `auth.json` in the per-user data directory
(`%APPDATA%\Nephele\` on Windows) — see §5. `.local/auth.json` is the legacy
location; `paths::migrate_legacy_file` copies it across once on launch, and
nothing writes there any more.

---

## 9. Protocol & online join

- `server_id_hash` reproduces Java's **signed `BigInteger.toString(16)`** — leading
  `-`, leading zeros dropped. Getting this wrong fails silently at the server.
- Order matters: **Mojang join call happens before** the encryption response is sent.
- `Connection::send_encryption_response` + `read_login_outcome`;
  `GameClient::join_online` runs the HTTP join on `spawn_blocking`.

---

## 10. Renderer

`crates/nephele-render/src/mesh.rs`

- `Vertex { position, uv, shade, layer }` — `shade` is a per-face constant
  (1.0 / 0.66 / 0.82 etc.), not a light value.
- `FACES: [FaceDef; 6]` describes each cube face.
- `mesh_column(&ColumnMeshJob, &TextureArray) -> MeshData` does the work;
  `build_column_mesh` / `build_world_mesh` wrap it.
- **Plants are crossed quads**, not cubes: `is_cross_plant(id)` covers
  6, 31, 32, 37, 38, 39, 40, 59, 83, 141, 142, 175, emitting two diagonals each drawn
  twice with opposite winding. Pinned by `plants_are_crossed_quads_not_cubes`.

### Shape-aware meshing — first pass done (2026-11-02)

`mesh_column` now consumes `nephele_world::shapes::block_shape(id, meta)`. Full
cubes keep the direct unit-cube path; each non-full `ShapeBox` emits its six faces
after converting its 0..16 pixel bounds to block fractions. Whole-face neighbour
culling runs only when both states are full cubes, preserving the existing
`is_transparent` rules without dropping the partially covered face beside a stair,
pane or other small shape. Cross-plants stay on their separate crossed-quad path.

The current table now renders slabs, stairs, fence posts, panes, door slabs and
standing torches. Fence rails and pane connections are not modelled yet; shaped
blocks currently emit their box faces without neighbour culling.

### Dropped items — done

`build_item_mesh(&[ItemDraw], &TextureArray, time)` in `mesh.rs` builds a
**0.25-block spinning, bobbing cube per stack** (reusing `FACES`, so the shading
matches blocks). It is drawn by the *terrain* pipeline and texture array — not
the entity pipeline, which is bound to the skin atlas — via
`Renderer::set_items(&MeshData)` and one extra draw call after the chunks.

- Block items use their own `[top, bottom, side]` textures.
- Mapped non-block items (sticks, crafted tools, armour and common food) use
  namespaced layers loaded from the prepared sibling `textures/items` directory.
  Other non-block item ids still use `MISSING_TEXTURE`.
- `ItemDraw.position` is the **feet** (block-bottom) position; the mesh adds
  `ITEM_HOVER = 0.1` and centres the cube on the block centre.

### First-person hand + held block — done (2026-11-02)

- `entity::append_hand(&mut EntityMesh, &HandDraw) -> HandPose` appends **one
  skin-textured arm box** (the right arm's `(40,16)` unwrap) to the entity mesh
  and returns where the palm ended up. `HandDraw` takes the camera's world-space
  basis, so the arm is built in camera space and placed in the world; it then
  renders in the normal depth-tested pass with the skin pipeline. No second
  render pass, so **the hand is occluded by walls** instead of drawing over them
  (vanilla clears depth for the hand) - acceptable for a placeholder, worth a
  separate pass later.
- `mesh::build_hand_item(&HandItem, &TextureArray)` builds the held **block**
  cube along the hand's axes; `live.rs` appends it to the same item mesh as the
  drops, at `HandPose::palm`, 0.36 blocks wide.
- `HandDraw.skin_slot` is `SkinAtlas::DEFAULT_SLOT` today: the built-in grey
  stand-in. Using the player's own skin needs the local player's atlas slot,
  which the render side does not track yet.
- The arm rests at the bottom-right and reaches forward-left; while the left
  mouse button is held (`FrameView.attacking`) the reach lifts and falls with a
  `sin`, which is the whole "swing" animation. `Camera::up()` was added for the
  basis.
- `draw_entities` now takes the whole `FrameView` (it needs `attacking`,
  `held_slot` and the inventory) and only attaches the hand on `Screen::Game`.


### Low-end renderer polish — done (2026-11-03)

Pillar 1's presentation/performance pass is in. It deliberately touched only
`nephele-assets`, `nephele-render`, the CLI render loops and UI — **no worldgen
or protocol changes**.

- **Animated textures.** `TextureArray` now reads each texture's sibling
  `.png.mcmeta`, honours `frametime` and the `frames` order, and packs every
  animation into one 2D strip atlas. The block shader uses the tick clock to
  choose a frame; water is 32 frames / 2 ticks, lava 38 frames (the mcmeta's
  palindrome), fire and portal are picked up from the same metadata. Covers are
  mapped in `block_textures`: water, lava (ids 10/11), fire (51) and portal (90).
- **Mipmaps.** The block array has the full 16 → 1 chain. Mips are generated in
  `nephele-assets` with an alpha-weighted box filter, so cutout edges do not
  darken. Sampling is nearest magnification with linear minification/mip
  transitions, matching vanilla's look.
- **Sky.** `sky.wgsl` is one full-screen ray pass: day/night gradient, square
  sun and moon, stable hashed stars, and two-octave procedural clouds at y=192.
  Time comes from the server's `TIME_UPDATE` in multiplayer and advances one
  tick per tick locally. `NEPHELE_TIME`, `NEPHELE_PITCH` and `NEPHELE_YAW` are
  viewer-only diagnostics for capturing a particular sky.
- **Fog.** `Environment` carries time, fog colour and start/end distance.
  `biome_fog_color` tints the horizon by the biome under the camera; being in
  water overrides it with short-range blue fog. Terrain and entities both apply
  distance fog in their fragment shaders.
- **Culling and streaming.** Frustum culling is unchanged. A conservative
  software occlusion pass rasterises each near column's 4×4 height field into
  an 80×45 depth buffer and only drops a chunk when all nine AABB samples are
  behind a nearer surface. It is disabled underwater/cave-level and can be
  turned off with `NEPHELE_OCCLUSION=0`. Finished column meshes now go through
  a **2 MiB / 12-column per-frame upload budget** instead of all uploading in
  the frame they complete.
- **Damage feedback.** `ui::draw_damage_vignette` draws a red edge gradient for
  ~0.55 s whenever health drops.
- **Measured.** `scripts/measure_perf.ps1` on the integrated AMD Radeon
  (Ryzen 5 7535HS), 1280×720, view distance 8, vsync off:
  frame p50 `6.960 ms`, p99 `7.410 ms`, 0 frames over the 16.667 ms budget,
  ~143 fps. A separate real-asset 30 s local-world run measured p99 `12.880 ms`.
  The normal-world screenshot checked during this pass is
  `C:\Users\dariu\AppData\Local\Temp\opencode\p1-final-world.png`.

### Smooth lighting + AO — done (2026-11-02)

`Section` decodes `block_light` and `sky_light` nibbles, `ColumnMeshJob` exposes
light lookups, and `Vertex` carries a per-vertex `light` value. `vertex_light`
samples the four tangent corners and applies the ambient-occlusion table; the
`vertex_lighting_averages_four_samples_then_applies_ambient_occlusion` test pins
it. Caves are dark, corners shade, and the shader multiplies the sampled texel
by `shade * light`.

---

## 11. Gameplay state

**Mining** is mode-aware and must stay that way:

- Creative: breaks on click START, with vanilla `HIT_DELAY_TICKS = 5`
  (one click = one block).
- Survival: accumulates damage and lets the server time it.
- `pending` / `pending_ticks` guard stops re-sending.

**Placement** must never put a block inside the player. `placement_hits_player` is
checked in **both** the multiplayer path and `local_loop` — the second one was missing
and produced the "pushing me up" bug.

**Physics** (`physics.rs`): sneak edge protection via `edge_clamped` + `has_ground`,
per-axis, inset footprint. `resting_on_the_floor_does_not_drift_upward` guards drift.

**Items and drops** (M2, first slice):

- `S0E Spawn Object` with `OBJECT_TYPE_ITEM = 2` becomes `EntityKind::Item { id, meta }`
  in `Entities`; the item id/meta are packed in the object's `data`
  (`item_object_data` / `item_from_object_data`). Other object types (arrows,
  falling blocks) are parsed and ignored.
- `S0D Collect Item` removes the collected entity. The inventory update arrives
  separately as `S2F Set Slot` / `S30 Window Items`, which the client already
  applied long before this slice — so pickup is **despawn + server-authoritative
  inventory**, never a client-side guess.
- The simulation is still 20 Hz: `step_player` is the only place physics runs.
- The mock server drops the block it breaks, collects it a second later and puts
  it in the first free hotbar slot, so the whole chain is testable without Java:
  `cargo run -p nephele -- mock-server --survival`, and the integration test
  `a_broken_block_drops_an_item_that_is_picked_up` in
  `crates/nephele-client/tests/join_mock.rs`. `--survival` also joins as
  game mode 0 so the survival mining path (not the creative one-click path) runs.
- Container clicks use `C0E Click Window` (`GameClient::click_slot`) with a
  per-window action counter; `S32 Confirm Transaction` comes back as
  `Event::ConfirmTransaction`, and `accepted = false` means the click was
  rejected and the slot will be resent. The packet reports **our copy** of the
  clicked slot, not the cursor stack: 1.8's server compares it with its own copy
  and rejects the click when they differ. Click modes live in
  `play::click_mode`; `SLOT_OUTSIDE (-999)` is the dropped-outside slot.
- **The mock implements the container** (`click_container`), so the whole
  inventory loop is provable without Java: normal left/right clicks (take,
  half, place all, place one, merge, swap), shift-click between hotbar and
  storage, and number-key swaps. Drag/double-click/middle-click modes are
  refused (`accepted = false`), which tells the client not to trust its
  prediction. A stale click (our copy of the slot differs from the server's) is
  also refused and the slot is resent.
- The mock's 2x2 recipes are **one log -> 4 planks** (plank meta follows log
  meta), **two planks in one column -> 4 sticks**, and **four planks -> a
  crafting table**. Stack sizes in the grid do not matter, matching vanilla.
  The shared `nephele_world::crafting` recipe book also includes 3x3 tools,
  furnace, chest and iron armour; its result is used by both the mock and local
  singleplayer.
- The mock now honours `C10 Creative Inventory Action` by writing the slot and
  echoing `S2F`, which is how tests (and `/give`) put a known stack in a slot.
  The integration test `a_log_in_the_crafting_grid_shows_planks_in_the_output_slot`
  covers the whole chain: C10 setup, two `C0E`s to move the log into the grid,
  and a third to take the result.

**Survival inventory screen** (added 2026-11-02):

- `E` in `Screen::Game` opens it when *not* creative; in creative the same key
  still opens the creative panel. `E`/`Esc` close it.
- Vanilla 1.8 layout in `ui::inventory_slot_origin` (panel 176x166, output at
  (144,36), 2x2 grid at (98/116, 18/36), armour down the left from y=8, three
  storage rows from y=84, hotbar at y=142). `inventory_slot_rect` turns any
  container slot into a screen rect, and the tests pin the positions plus
  "inside the panel, never overlapping".
- Left click takes all / places all / merges up to 64 / swaps; right click takes
  half / places one. Shift-click quick-moves between hotbar and storage. The
  rules live in `crates/nephele-cli/src/inventory.rs` (client half) and are
  mirrored by the mock's `click_container` (server half).
- **Prediction split:** the cursor stack is pure client state; slots stay
  server-authoritative. In single-player (`Shared.local`) the same helpers also
  write the slots, since there is no server. Against a server the click is queued
  into `Shared.inventory_clicks` and sent as `C0E` with `clicked` = our copy of
  the slot before the click.
- Closing while holding a stack puts it back in the first free hotbar/storage
  slot (a click) instead of vanilla's drop-on-the-ground.
- **Local crafting (added 2026-11-02).** `inventory::crafting_result` delegates
  to the shared `nephele_world::crafting` recipe book. The player inventory
  maps its 2x2 grid into that matcher; a placed crafting table opens a local
  46-slot window with the 3x3 grid. Output clicks consume one item per occupied
  cell only if the full result fits on the cursor; shift-click crafts repeatedly
  while ingredients and inventory space remain. Closing a local table returns
  its grid and cursor atomically, preserving items if the inventory is full.
- **Local survival block interaction (added 2026-11-02).** Breaking blocks awards
  a basic block item directly into hotbar/storage (stone -> cobblestone, grass ->
  dirt, logs preserve species but discard axis bits); survival block placement
  decrements the selected stack. These local drops are not yet physical item
  entities and the mapping is not a complete vanilla drop table.
- **Crafting-table window works end to end (2026-11-02).** Right-clicking a
  placed crafting table with a non-block/empty hand sends `C08` with an empty
  held slot; the mock answers `S2D` (window id 1, `minecraft:crafting_table`,
  46 slots) + `S30`, and `C0D` closes it, returning the grid and the cursor
  stack to the player. The window's storage/hotbar half (slots 10..=45) mirrors
  the player inventory, so the panel is the same one with a 3x3 grid on top.
  The mock's 3x3 recipes: planks, sticks, crafting table, torch, furnace, chest,
  pickaxe/axe/shovel/sword in wood/stone/iron, and iron armour. Patterns are
  matched after trimming empty rows/columns, so a shape can sit anywhere in the
  grid; stack sizes in the grid do not matter. Local singleplayer uses the same
  recipe matcher; a real multiplayer server still computes its own output.
- **Item icons (added 2026-11-02).** `TextureArray::load_from_dir` appends
  namespaced layers from the prepared sibling `textures/items` directory, so
  item names cannot collide with block layers. Inventory slots render mapped
  tools, armour, materials and common food as flat icons; dropped-item cubes use
  the same item texture on each face. Unmapped item ids still use
  `MISSING_TEXTURE`, and held first-person rendering remains block-only.

**Chat and client-side commands** (added 2026-11-01):

- `T` opens the chat line, `/` opens it with a slash prefilled, `Enter` sends,
  `Esc` closes. While it is open it owns the keyboard, the mouse is released and
  movement keys are dropped; the HUD's "click to look around" hint is suppressed
  via `HudState.chatting`.
- Recent chat is drawn bottom-left and fades after 10 s (`draw_chat` in `ui.rs`,
  14 s hard expiry in `LiveApp::draw_chat`). Colour codes are **not** parsed yet —
  chat arrives as plain text.
- `commands.rs` parses `/help`, `/gamemode`, `/tp`, `/setblock`, `/give`, `/seed`.
  `~` relative coordinates and `~5` offsets work; `setblock`/`give` take block
  names through `nephele_world::blocks::by_name` (names are per-variant:
  `red_wool`, not `wool:14`).
- **Commands are client-side.** That is a testing decision, not parity: the mock
  and singleplayer have nothing to execute them. A command the module does not
  recognise is forwarded to the server unchanged, so real server commands still
  work. Effects travel through `Shared` (`chat_out`, `world_edits`,
  `creative_override`); `/give` uses the creative-slot packet, so it needs
  creative mode.
- `local_loop` now reads `Shared.creative` instead of hardcoding creative, and
  `start_local` sets it true, so `/gamemode survival` changes mining timing in
  singleplayer without breaking the old default.

---

## 12. Milestones

**The M-numbers are retired** — they mean three different things in three
places (`Future.md`'s milestones, the old session names, and the roadmap), and
`M5` alone meant both "world saves" and "the product". Lanes are named now:

| Lane | Owns | Branch | State |
|---|---|---|---|
| `render` | shapes, tinting, sky, fog | `render` | 95%: bars, fire, plant culling and biome tint landed; audit leftovers |
| `biomes` | GenLayer parity | `biomes` | **100% biome cells, every stage diff 0 — lane complete** |
| `biome-vis` | singleplayer generator + biome data path | `biome-vis` | done: `local_loop` streams `VanillaGenerator`, columns carry `Level.Biomes`; needs merge |
| `worldgen` | two-stage generation, caves, ores, flora, structures (W1–W5) | `w-worldgen` | split done; population generators next (80% of block diffs) |
| `survival` | inventory, containers, HUD art | `survival` | 90%: server windows sized from the packet, HUD sprites + armour row |
| `net` | protocol, real servers, maps, frames, metadata, scoreboards | `net` | 95%: scoreboards, teams, `C02`, maps and frames all parsed |
| `client-ui` | chat colours, settings, loading screen, modules | `client-ui` | 20%: worktree exists, nothing landed yet |
| `saves` | `level.dat`, player data, region writing | `saves` | 90% and **landed on main** |
| `entities` | players, skins, nametags, frames, maps on screen | `entities` | 60%: head yaw, holograms, picking, lobby population |
| `perf` | startup, view distance, baseline gate | `perf` | 100%, committed baseline; needs merge |
| `audio` | everything sound | `audio` | 35%: symphonia + kira, break/place/step/hurt |
| `product` | launcher v2, updater, installer, legal | `saves-landing` (WIP commit) | 25%, blocked on code signing |

Rules that hold across all of them: one worktree per lane (never edit the main
checkout), its own `target/`, and `scripts/run_client.ps1 -Session <lane>` for
in-game testing — two clients on one machine fight over the window and the
single Microsoft account. `net`, `entities`, `survival` and `render` all touch
`live.rs`, so no more than one of those should be mid-edit at a time.

**M1 status (2026-11-03):** the shape table is visible through the mesher for
slabs, stairs, fence posts, panes, doors and torches. Animated `.mcmeta`
textures, mipmapped filtering, procedural sky/clouds/stars, per-biome fog,
frustum + height-field occlusion culling, the per-frame upload budget and the
damage vignette are in. Remaining M1 work is broader plant/terrain edge cases.

**M2 in progress.** The fixed 20 Hz tick, server-driven health/food/XP data path,
network block drops and pickup, survival inventory, container click protocol,
mock 2x2/3x3 crafting, and the death screen and respawn are implemented. Local
singleplayer now evaluates the shared recipe book in both the 2x2 inventory grid
and a usable 3x3 crafting-table window, awards basic block items when mining, and
consumes block stacks when placing in survival. The tree-to-pickaxe chain still
needs an interactive hand check. **Still missing:** eating, armour equip and
damage reduction, hunger effects, XP orbs, furnace/chest block entities, and
item icons for unmapped item ids.

**M5 singleplayer saves (2026-11-04).** World storage is vanilla-shaped now:
`nephele.dat` remains as a compatibility mirror, but `level.dat` is the source
of truth and carries `Data.Player` (position, rotation, health, hunger, XP,
hotbar slot, bed spawn, inventory) plus `Time`/`DayTime`/rain/thunder and the
world spawn. `level.dat` is written through a temp file and a rename, with the
previous complete file kept as `level.dat.bak`; a corrupt primary falls back to
the backup on open. Region writes pack every live record into a replacement
`.mca` and rename it into place, so a process killed mid-write leaves the old
region intact; unmodified records keep their sector offset and timestamp.
Unknown `level.dat` tags (gamerules, border settings, world-gen options) survive
a save: verified against a real 1.8.9 server world, 35 unknown tags unchanged.
The private 1.8.9 server opened a Nephele-created world, kept a gold block we
placed, and advanced the saved clock/weather. Chunk streaming unloads columns
beyond radius + 2 (saving dirty ones first), and autosave runs every 15 s and on
quit. Day/night is the renderer's `Environment` shader; rain and lightning are
CLI screen-space overlays, so the M1/M2 renderer stays untouched. **Not
verified:** a ten-minute live walk with memory profiling, and a real
vanilla-client (not just server) load of `Data.Player`. **Known gap:** rewriting
an imported chunk regenerates its NBT from terrain only, so `Entities`,
`TileEntities` and `TileTicks` are dropped; Nephele's own worlds never create
those, but a future vanilla-world importer must merge them rather than
re-serialize the chunk wholesale.

---

## 13. Gotchas that have bitten this project

**PowerShell**

- `.Replace()` with CRLF fails on LF files → use the `edit` tool.
- `[System.IO.File]::ReadAllText` with a relative path resolves against the process
  CWD, not the PS location → always use absolute paths.
- `-replace` needs `${1}_${2}` — `$1_$2` silently misbehaves.
- `Set-Content -Encoding UTF8` writes a BOM → use
  `New-Object System.Text.UTF8Encoding($false)`.
- Never put backticks inside a double-quoted bash string.
- `cmd1; cmd2` does **not** stop on failure — a clippy error once rode into a commit
  this way.

**Worldgen**

- `getBiomesForGeneration` reads `initializeAllBiomeGenerators()[0]` — the coarse
  **1:64 RiverMix layer** — not the 1:1 voronoi map. Getting this wrong was a 50.7%
  error; fixing it dropped it to 4%. Then terrain height converged to 96%.
- `NoiseGeneratorSimplex`'s 2D path uses `xCoord` + **`yCoord`**, not `zCoord`.
- `func_151607_a` truncates toward zero then subtracts 1 — it returns **-1** for 0.0.
- `JavaRandom::next_boolean` consumes **1 bit**; `nextInt(2)` is different.
- Mesa `func_180629_a` samples the noise with **x on both axes** — a vanilla bug that
  must be preserved, not fixed.
- Caves' water check is on the shell only; the quirk is load-bearing.
- Veins centre at `pos+8`, so the ore pass must replay the **9 surrounding chunks**;
  `decoration_seed = x*341873128712 + z*132897987541`.
- **Empty lanes … (n/a here)** — that's a Roblox note, ignore.
- Biome mutations have height `(0.1, 0.2)`; missing ids 14/15/28 had to be added to a
  `[Biome; 40]` table.
- Hills surface rule is three rules, not one: mutations get gravel when noise
  ∉ (−1..=2); stone appears above noise 1.0 unless the biome is Extreme Hills+ (34).
- **Generated columns must carry `Level.Biomes`.** `ChunkColumn::set_biomes` is
  what `biome_at`, `ColumnMeshJob`, the save writer and `biome_at`'s fog read.
  Every one of them falls back to plains when it is absent, so a missing map
  looks like "this world is all plains", not like an error.
- `GenLayerHills` never seeds its variance layer, and that is correct. Vanilla
  keeps the hill-noise layer in `field_151628_d`, outside `parent`, so
  `initWorldGenSeed` never reaches it and the two zooms above it run with
  `worldGenSeed == 0` for the whole world. Seeding it changes the
  `(noise - 2) % 29` tests and dropped the biome map from 100% to 92.8%. Pinned
  by `hills_never_seeds_its_variance_layer`; do not "fix" it.

**Blocks**

- `blocks.rs` is generated from Leafish's `legacy.rs` (MIT/Apache, attributed) — 1,374
  states / 228 ids. `name(id)` answers the **metadata-0 variant**, so `name(31)` is
  `"dead_bush"`. Use `VARIANTS` when metadata matters.

**World saves (1.8.9)**

- Singleplayer player data lives in `level.dat` at `Data.Player`, not in a
  `playerdata/` file. `Pos` is feet position, `Rotation` is `[yaw, pitch]` in
  vanilla degrees (yaw 0 = +Z, pitch positive = down), so the client converts
  with `+180°` and a pitch sign flip.
- Inventory `Slot` is 0–8 hotbar, 9–35 main, 100–103 armour (100 = boots). The
  parser maps those onto the client's 36–44/9–35/5–8 container slots; anything
  outside the mapping is preserved untouched so modded items survive a save.
- An empty NBT list is written as element type `0` followed by a length of `0`.
  That is what vanilla 1.8.9 writes and reads — do not "improve" it into a
  missing length.
- `level.dat` is gzipped NBT with a `Data` compound. Keep unknown tags inside
  `Data` (and root fields outside it) in the `IndexMap`; dropping `GameRules`
  or the border/world-gen tags corrupts worlds people care about.
- The previous complete `level.dat` is `level.dat.bak`; `level.dat_old` is a
  vanilla fallback. `read_level_document` tries all three before giving up.
- Region saves are whole-file replacements (temp + rename), not in-place sector
  edits: a crash must never leave a truncated `.mca`.

**Tooling**

- `.gitattributes` sets `* text=auto eol=lf`. Keep it.
- Git identity is repo-local: `Casualdv <Casualdv@users.noreply.github.com>`.
- `project_root()` walks up from the executable and prefers a `[workspace]` Cargo.toml
  over a stray `.local/prepared` — that stray-directory case used to break asset
  lookup from any other cwd.

---

## 14. Testing philosophy

- Parity claims are **measured**, never asserted. Every number in §6 came from a run
  against the reference world.
- Reference-world checks live in `genlayer.rs` and `worldgen.rs`;
  `chunk_biomes_match_a_world_the_real_server_generated` passes and
  `debug_first_mismatch` / `debug_stage_trace` narrow any future regression.
- **536 test cases across the workspace** (529 passed, 7 ignored). `nephele-world`
  carries the parity suite, including `debug_stage_trace` and
  `debug_first_mismatch`. The six real-socket tests in
  `crates/nephele-client/tests/join_mock.rs` cover status,
  compression, chat, joining/loading the mock world, drop pickup, and crafting
  planks through `C0E`.
- A test that pins a quirk (signed hex, `func_151607_a`, x-on-both-axes) should say so
  in a comment, or someone will "fix" it later.

---

## 15. Next move

**Landed:** M1's rendering half, M5's world saves, M6's instrumentation.
`block_shape` now drives the mesher - slabs, stairs, fences, panes that connect
across chunk borders, doors with paired metadata and yaw placement, torches -
and vertices carry sky/block light with ambient occlusion. Singleplayer creates,
loads and saves dirty chunks under `%APPDATA%\Nephele\saves`. Performance metrics
are opt-in behind `NEPHELE_PERF=1`, with a reproducible run via
`scripts/measure_perf.ps1`.

**Pillar 1 renderer polish landed 2026-11-03** (branch `p1-render`): animated
`.mcmeta` textures, mipmapped block filtering, procedural sky (sun/moon/stars/
clouds), per-biome fog, conservative height-field occlusion culling, the
per-frame chunk upload budget and the damage vignette. Measured p99 on the
integrated Radeon is 7.41 ms at 1280×720 / view distance 8 with vsync off.

**Open:**

1. **M2 - offline survival.** The exit criterion was played end to end offline: logs, planks, sticks, crafting table, then a wooden pickaxe with correct counts and no item loss. Remaining polish only: eating, armour effects, XP orbs, block entities, item-atlas wiring.

2. **M3 - join a real server.** Dimension change, keepalive, tab list, and a
   shippable Azure app id.
3. **M6 follow-up.** Attribute the startup frame spikes by stage, and report
   every measurement window instead of the best one.

**Landed 2026-11-05: M1b - biome parity, 100%.** 0 of 160,000 reference blocks
differ (was 11,492). The fault was `GenLayerHills` reseeding its variance layer —
see §6. Negative results worth keeping: a ±1-block shift made it worse;
bypassing `RareBiome`, `AddIsland` or `Shore` changed nothing; bypassing
`Smooth` or `RiverMix` made it worse. That correctly pointed at the 1:64 Hills
output. `GenLayerHills`/`GenLayerRareBiome` arithmetic itself was never wrong.

## 16. Working agreement

**Which doc gets updated, and how much:**

| File | When | How much |
|---|---|---|
| `AGENTS.md` | always, when behaviour changes | as much as staying accurate needs |
| `Future.md` | when an item lands, changes shape or is dropped | tick it, note the status change |
| `README.md` | when a **user-visible** feature lands | one line in "Working", nothing more |
| `docs/`, `BUGS.md`, anything else | explicit request only | asked-for scope only |

Rationale: a stale roadmap is worse than none, and a README that undersells the
product is a sales problem — but neither is a place to narrate internals. This
file is the only one that carries the full story.
- Don't re-litigate settled decisions without new evidence: the compliance model, the
  licence table, the `IntCache` caveat, and the scale model are all settled by evidence
  already gathered.
- Prefer measurement to reasoning about Minecraft internals. Every time I reasoned from
  the decompiled source alone, I was wrong; every time I measured, I found the bug.
- Commit only when the tree is green: fmt, clippy, tests, publish gate.
