# AGENTS.md — Nephele Client

Read this before touching anything. It is the durable memory of the project: every
session starts here, and it is the only file that should be updated by default as part
of a task. Keep it accurate when you change behaviour.

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

Environment: **overworld, sea level 63**, infinite generation, biome map from the
1.8 `GenLayer` stack.

### Parity status (measure against the reference world, never by eyeball)

| Stage | State | Number |
|---|---|---|
| Terrain height field | done | 96% of columns match |
| Surface blocks | done | 98.8% match (1.2% mismatch) |
| Caves | done | 308 air/chunk below y=56 |
| Ravines | done | 426 vs 435 reference (98%) |
| Ores | done | 5–12% above reference (caves were absent when measured) |
| **Biome map** | **stuck** | **92.8%** |
| Features | partial | flowers, grass, dead bush, cactus, oak-shape trees |

### The biome bug — where it stands

The divergence is **downstream of Hills**, somewhere in
`RareBiome → Zoom×4 (AddIsland, Shore) → Smooth → RiverMix`.

**Corrected scale model** (an earlier revision of my notes was wrong; trust this):

- `GenLayerBiome` → `BiomeEdge` → **Hills are 1:64**
- RareBiome → 4× Zoom (AddIsland at k=0, Shore at k=1) → Smooth → RiverMix are **1:4**
- Voronoi reads its parents at `(block - 2) >> 2`, output 1:1

`initChunkSeed` and `nextInt` are **verified byte-identical to vanilla** — four
multiply-add steps alternating x/z, and `nextInt` advancing with
`chunkSeed*M+A + worldGenSeed`. They are not the bug. Do not re-audit them.

Finding the remaining 7.2% requires changing the Rust and measuring against the
reference world. Cell-by-cell oracle comparison is a trap (see §7).

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

**Critical caveat: the oracle is only trustworthy at the final map.** Stage-level
queries are corrupted by `IntCache` aliasing — Java's `hillsOnly` contradicts its own
`hillNoise` in the same window. I proved this; it invalidated a whole round of
"stages match" conclusions. Compare per-stage results against the **reference world**,
never cell-by-cell between two oracle stages.

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

**Still cubes, still wrong:** slabs, stairs, fences, panes, torches, and every other
non-full-cube shape. Fixing those needs a real per-state vertex-box model system —
bigger than everything else in M1 combined.

### Smooth lighting — planned, not started

Light **is** decoded: `Section` stores `block_light: Vec<u8>` and
`sky_light: Option<Vec<u8>>`, nibble-packed (`SECTION_LIGHT_BYTES`), with accessors.
What's missing:

1. `ColumnMeshJob` exposes only `block(x,y,z)` — it needs a `light(x,y,z)` reading the
   nibbles out of the section.
2. `Vertex` has no light field; bytemuck layout and `shader.wgsl` both change.
3. The sampling — 4 corners per vertex plus ambient occlusion, which is what makes
   caves dark and corners shaded. Per-face light first, AO second.

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

---

## 12. Milestones

M0 bootstrap → **M1 worldgen + renderer** → M2 survival loop → M3 multiplayer polish →
M4 PvP features → M5 performance → M6 release. Details in `Future.md`.

**M1 remaining:** block shapes (cross-plants done; slabs/stairs/fences panes/torches
not) and smooth lighting. Then M1 is closed.

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

**Blocks**

- `blocks.rs` is generated from Leafish's `legacy.rs` (MIT/Apache, attributed) — 1,374
  states / 228 ids. `name(id)` answers the **metadata-0 variant**, so `name(31)` is
  `"dead_bush"`. Use `VARIANTS` when metadata matters.

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
- Reference-world checks live in `genlayer.rs` and `worldgen.rs`; the biome one
  (`chunk_biomes_match_a_world_the_real_server_generated`) is the active failing one,
  with an ignored `debug_first_mismatch` helper for narrowing it.
- **109 tests** in `nephele-world`; **375 across the workspace** (1 ignored);
  13 in the server list (9 list + 4 ping board), plus 9 `ui` tests for the
  multiplayer screen's buttons, columns and layout, and 3 for the paths module.
- A test that pins a quirk (signed hex, `func_151607_a`, x-on-both-axes) should say so
  in a comment, or someone will "fix" it later.

---

## 15. Next move

1. ~~**Server list screen**~~ — done, and now with live pings. How it is wired is
   in §5 ("The multiplayer screen"); the short version: saved list + async
   ping/MOTD/version columns, select-then-join, Direct Connect, Refresh, and the
   list persists to `servers.json` in the user data directory. Still missing from
   it: **player counts** (`online`/`max` are in the ping response but `servers::Status`
   drops them), a history list distinct from the saved list, and drag-to-reorder.
2. **World creation screen** — smaller. `local_loop` already takes a seed and
   `play --seed` already parses one; v1 creates but does not save (region writing is
   M2).
3. Then smooth lighting, then slabs/stairs, closing M1.

---

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
