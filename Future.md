# Nephele — Future

A living plan. Everything below is tagged with a rough size: **S** (under a day),
**M** (a few days), **L** (a week or more), **XL** (multi-week, needs its own
design pass). Re-order freely; the point is that nothing important is forgotten
and that the big calls are written down with their reasons.

---

## Target product

A legitimate 1.8.9 PvP client in Rust, released like Lunar / Feather / Badlion:
a launcher plus a fast client that runs well on **low-end laptops** and older
integrated graphics. That target drives three decisions that would otherwise
look odd:

- **Performance is a feature**, not a nice-to-have. 60 fps on Intel HD graphics
  is the bar, not 300 fps on a 4090.
- **OpenGL matters.** Lunar, Feather and Badlion are all GL. Vulkan/DX12 drivers
  on old laptops are frequently broken or missing. See "Open decisions".
- **Online-mode servers must work.** A PvP client that cannot join
  `hypixel.net` has no users.

Nephele ships **no Mojang code or assets**. Resources are built from the player's
own 1.8.9 jar by `nephele-prepare`, verified against `research-lock.json`.

---

## Reference projects

| Project | Licence | What it is | What we may do |
|---|---|---|---|
| `Thinkofname/steven` | MIT / Apache-2.0 | The original 2015 Rust client | Copy with attribution. Superseded. |
| `iceiix/stevenarella` | MIT / Apache-2.0 | Actively-kept fork of steven | **Copy with attribution** |
| `Lea-fish/Leafish` | MIT / Apache-2.0 | Furthest-along fork: entities, inventory, screens, lighting | **Copy with attribution** |
| `azalea-rs/azalea` | **MIT** | Full Rust MC stack: auth, crypto, protocol, world, entity, inventory, brigadier | **Copy with attribution. Highest value.** |
| `Oery/origami` | MIT | ~1.4k-line pure-Rust bot; very readable event/packet layer | Copy with attribution |
| `cohaereo/minecrab` | **GPL-3.0** | Rust bot/automation | **Read only.** |
| `Exopteron/opengl-classic-client` | none stated | Toy Rust/OpenGL Classic client | **Do not copy** — no licence |
| `PommeMC/Client` | **GPL-3.0** | Most complete client (265 files): auth, audio, singleplayer | **Read only.** Never copy. |
| Our own oracle (`Nephele-private`) | private | Decompiled 1.8.9 + Java harness | Never published |


### Licence rules

- MIT/Apache code may be copied into this repository. Keep the original
  copyright lines and add a `NOTICE` crediting Thinkofdeath, iceiix and
  terrarier2111.
- **GPL-3.0 (`PommeMC/Client`) cannot be linked into a proprietary binary.**
  Reading it to understand a protocol is fine; copying code is not. Anything we
  learn there must be re-implemented from Microsoft's public documentation and
  our own oracle.
- Never vendor decompiled Minecraft into this repository. `Nephele-private` is
  private and stays that way.

### What the references actually contain

- **None of them generate 1.8.9 terrain.** Leafish and stevenarella have a
  119-line biome colour table and nothing else. Our worldgen is already ahead.
- **Azalea solves authentication.** `azalea-auth` (MIT) implements the full
  Microsoft device-code flow, token refresh, ownership check, profile fetch and
  session-server join; `azalea-crypto` has the AES/RSA online-mode handshake.
  It targets modern versions, but auth and the join handshake are the parts that
  did not change. This is a port, not a research project.
- Azalea also carries `azalea-brigadier` (command parsing and suggestions),
  `azalea-chat` (JSON chat components), `azalea-inventory`, `azalea-entity` and
  `azalea-physics` — all MIT, all useful as structure even where 1.8.9 constants
  differ.
- The useful, borrowable material in the older clients is: Leafish `entity/`
  (19 files), `inventory/` (24), `screen/` (12), `world/lighting.rs`,
  `entity/block_entity/`, `particle/`.
- `minecrab` and `PommeMC` are GPL-3.0: read for understanding, never copy.


---

## Where we are today

Working: protocol 47 handshake / login / status / join, **live play on a real 1.8.9
server** (Hypixel, 169 chunks, no desync), chunk codec + light, wgpu renderer,
worker-pooled meshing, player skins, bitmap font + UI atlas, title / pause /
settings / connect / creative screens, HUD (hotbar, crosshair, health, hunger,
armour), physics (walk, sprint, sneak, fly, jump, sneak edge protection, slime
bounce), mining + placement, creative inventory, anvil **reading and writing**,
vanilla-compatible world saves (level.dat with player state, unknown-NBT
preservation, atomic writes with backups, region compaction, weather), biomes +
density height field + surface pass, shape-aware block meshing (slabs, stairs,
fences, panes, bars, doors, torches, fire, cross-plants with correct culling),
animated textures, mipmapped filtering, procedural sky and per-biome fog, grass
tinting, frustum + height-field occlusion culling, a per-frame upload budget,
damage vignette, **scoreboards + teams + chat**, **maps, item frames and entity
metadata**, players with head yaw and armour-stand holograms, **audio** (symphonia
decode, kira mixing, break/place/step/hurt), launcher, CI, verified-resource
preparation. Performance: 143 fps / p99 7.4 ms on integrated AMD at 1280×720,
view distance 8, vsync off, with a committed baseline that fails the build on
regression. The launcher now has per-profile game directories and window sizes,
a first-run setup screen that reports a failed `nephele-prepare`, local crash
reports with opt-in-only upload, an in-app legal screen, and a signed update
*check* that stays disabled while the signing key is empty.

Measured parity against a world the real 1.8.9 server generated: **100%** of
biome cells (0 / 160,000, seed 12345, 625 chunks — every GenLayer stage diff 0),
**96%** of surface heights, **98.8%** of surface block types.

---

## Pillars

### 1. Renderer and low-end performance

| Item | Size | Notes |
|---|---|---|
| Decide the backend: wgpu vs OpenGL vs both | **XL** | See open decisions. Blocks almost everything else. |
| GLES/Vulkan fallback path or a GL renderer | XL | Only if the above says "support old drivers". |
| Smooth lighting + ambient occlusion | L | We decode light but do not apply it per vertex. Biggest visual win. |
| Block shapes from vanilla models (stairs, slabs, fences, panes, cross plants) | L | The mesher now emits the `block_shape` boxes for slabs, stairs, fence posts, panes, doors and torches. Cross plants remain crossed quads. Fence rails and pane connections are still absent, and shaped boxes are not neighbour-culled yet. |
| First-person hand + held block | M | Done: skin-textured arm, held block, swing. Occluded by walls (no depth-clear pass). |
| Animated textures (`.mcmeta`, water/lava/fire/portal) | M | **Done 2026-11-03:** frame strips in a 2D atlas, tick clock in the block shader. |
| Sky: sun, moon, stars, clouds, day-night, fog per biome/dimension | L | **Done 2026-11-03:** full-screen ray sky, server TIME_UPDATE, per-biome and underwater fog. |
| Frustum + occlusion culling | M | **Done 2026-11-03:** frustum plus a conservative 4×4 height-field depth buffer; `NEPHELE_OCCLUSION=0` disables it. |
| Chunk upload budget per frame; async mesh results already exist | M | **Done 2026-11-03:** 2 MiB / 12 columns per frame. |
| Mipmapping / texture filtering parity with vanilla | S | **Done 2026-11-03:** full 16→1 chain, alpha-weighted, nearest mag + linear mips. |
| Held item in first person + swing animation | M | PvP clients care about this. |
| Damage overlay, hurt flash, vignette | S | **Done 2026-11-03:** red edge vignette on health loss. |

### 2. World generation

| Item | Size | Notes |
|---|---|---|
| **Mesa** stratified hardened clay | M | The last missing biome override. |
| Caves (`MapGenCaves`) and ravines | L | Needed before the world feels real underground. |
| Ores + granite/diorite/andesite, dirt/gravel/clay pockets | M | |
| Trees (6 species + large variants), flowers, tall grass, cacti, sugar cane, lily pads, pumpkins, melons | L | |
| Lakes (water/lava), dungeons, spawners | M | |
| Snow and ice, podzol, vines, sand patches | S | |
| Structures: villages, mineshafts, strongholds, temples, witch huts, monuments | XL | |
| World types: superflat, large biomes, amplified, customized JSON | M | |
| Nether and End generation, portals | XL | Out of scope for a PvP client v1. |
| Chunk generation on worker threads, streamed around the player | M | Singleplayer only; servers send chunks. |

### 3. The tick loop, movement and physics

**Fixed tick: done.** Physics runs at a fixed **20 ticks/second** (`TICK = 50 ms`,
`step_player`) from both the network and single-player loops, and the renderer
interpolates between ticks with `alpha`. The earlier note here said physics ran
per render frame — it no longer does. Keep new simulation work inside
`step_player`, never in the render loop.

| Item | Size | Notes |
|---|---|---|
| 20 Hz fixed tick + interpolation | **done** | `step_player`, `TICK`, `alpha` interpolation. |
| Movement constants parity | S | Walk `0.1`, sprint `×1.3`, sneak `×0.3`, gravity `0.08/tick²`, drag `0.98`, ground friction `0.546`, air `0.91`, jump `0.42`, terminal `3.92`, width `0.6`, height `1.8` (sneak `1.65`), eye `1.62`, reach `4.5`/`5.0` |
| Sprint rules: only forward, not while sneaking, cancels on hit | S | |
| Sprint FOV modifier | S | |
| Ladders, vines, cobwebs | M | |
| Water and lava movement, swimming, buoyancy | M | |
| Ice, slime blocks, soul sand | S | |
| Fall damage, drowning, fire, cactus | M | |
| Boats and minecarts riding | M | |
| Elytra | — | Not in 1.8. |
| Burst movement / anti-cheat-safe packet cadence | M | Vanilla sends a position packet every tick plus when moved; many servers flag anything else. |
| Tick-accurate block break timing in survival | M | Server is authoritative; mirror its timing. |

### 4. Entities, mobs and items

| Item | Size | Notes |
|---|---|---|
| Mob models + walk animation (humanoid, quadruped, creeper, slime, squid, bat, chicken, spider, enderman) | **L** | Port from Leafish `entity/`. |
| Entity metadata: equipment, nametags, invisibility, crouch, arm swing, health | M | |
| Dropped items, XP orbs, arrows, fireworks, paintings, falling blocks | M | **Block drops done**: `SPAWN_OBJECT`/`COLLECT_ITEM` tracked, rendered, collected into the hotbar. XP orbs, arrows and the rest are parsed but not drawn. |
| Item entities rendering with correct spin/bob | **done** | `build_item_mesh`: a 0.25-block textured cube, spinning and bobbing. Non-block items draw the missing-texture tile until the item atlas exists. |
| Slim (Alex) player model, capes | S | |
| Player nametags above heads | S | |
| Particles (block break, splash, smoke, flame, crit, portal) | M | Port from Leafish `particle/`. |
| Block entities: signs, chests, banners, beacon beam, enchanting table book | L | Port from Leafish `entity/block_entity/`. |

### 5. Inventory, containers and survival

| Item | Size | Notes |
|---|---|---|
| Survival inventory screen (2×2 crafting, armour slots) | L | Done: vanilla layout, cursor stack, left/right/shift clicks via `C0E`. |
| Crafting table, furnace, chest, hopper, dispenser, brewing, enchanting, anvil, beacon | XL | **Crafting table done** (server window protocol plus a local 3×3 window); furnace/chest and the rest unstarted. |
| Shift-click, drag-split, item dragging, hotbar swap keys | M | |
| Item tooltips: name, rarity colour, enchantments | M | |
| Durability bar, stack counts, item icons | M | Partial: prepared icons now render for mapped tools, armour, materials and common food; unmapped ids still use the missing tile. |
| Crafting recipe matching | L | Partial: a shared recipe book drives local 2×2/3×3 grids and the mock's 3×3 recipes, including the tree-to-pickaxe chain. The mock's 2×2 recipes remain separate; most vanilla recipes and container recipes are absent. |
| Eating, potions, status effects | L | |
| Enchantments | L | |
| Health, hunger, saturation, XP, air supply | M | Server-driven; we render and predict. |
| Death screen, respawn, keep-inventory | M | |
| Armour bar + armour slots | S | |

### 6. HUD and PvP quality-of-life

This is what separates a PvP client from a compatible one.

| Item | Size | Notes |
|---|---|---|
| Hunger / XP / armour / air HUD | M | |
| Hotbar selection highlight, item counts, durability | S | |
| Chat overlay + input + command suggestions | M | **Overlay + input done** (fade, `T`/`/`, Enter/Esc). Client-side `/help /gamemode /tp /setblock /give /seed`; unknown commands go to the server. No tab-completion or chat colours yet. `azalea-brigadier` and `azalea-chat` are MIT references. |
| Scoreboard sidebar, tab player list, boss bar, titles | M | |
| **Client modules:** FPS, CPS, keystrokes, coordinates, ping, armour status, potion timers, reach display | M | The Lunar/Badlion basics. |
| **Toggle sprint, toggle sneak** | S | |
| **Zoom** (OptiFine-style smooth zoom) | S | |
| **Custom crosshair, hit colour, hit delay indicator**, block outline toggle | M | |
| **Motion blur off, brightness/gamma slider, fullbright** | S | |
| Freelook / 360 view | S | |
| Nametag scaling, entity hurt tint | S | |
| Settings screen: keybinds, GUI scale, render distance, particles, graphics, language, resource packs | L | |
| Debug screen (F3) parity | M | |

### 7. Audio

No audio exists yet.

| Item | Size | Notes |
|---|---|---|
| Sound engine + positional playback | L | `symphonia` for Ogg Vorbis decode. |
| Block, step, break, place, mob and player sounds | M | |
| Music and records | M | |
| Volume sliders, subtitles optional | S | |

### 8. Multiplayer and authentication

| Item | Size | Notes |
|---|---|---|
| **Microsoft device-code OAuth → Xbox Live → Minecraft services** | **done** | `nephele-auth` + `nephele auth`; `play --online` uses the cached session. Still needs an Azure client id. |
| Token storage + refresh | M | `azalea-auth` has a cache; encrypt at rest. |
| Online-mode encryption handshake | **done** | AES-CFB8, server id hash, RSA PKCS#1 and the session-server join all implemented. |
| Server list screen (ping, MOTD, version, favourites, history) | **done** | Saved list with a live ping per row (MOTD, version, latency, 1s timeout, refreshed on open and by a Refresh button), favourites, select / double-click / Enter to join. No history list and no player counts yet. |
| Dimension change, respawn packets, portals | L | |
| Resource packs (server-sent) | M | |
| Full C→S action coverage (entity action, use entity, spectate, plugin channels) | M | |
| Disconnect reasons, reconnects, login errors | S | |

### 9. Singleplayer and world management

| Item | Size | Notes |
|---|---|---|
| Region file **writing** (`.mca`), `level.dat`, player data, session lock | L | Done for 1.8.9: `level.dat` (`Data.Player`), atomic whole-region compaction with per-chunk timestamps, and a `.bak` of the previous `level.dat`. Verified by opening a Nephele world in the private 1.8.9 server, which kept a placed gold block. `session.lock` is not written. |
| World creation screen (seed, name, mode, world type) | M | Name, seed and mode; world type is fixed to `default`. |
| World list, rename, delete, backup | M | Rename/delete plus timestamped snapshot and restore from the world list. |
| Autosave, save-on-quit, dirty chunk tracking | M | 15 s autosave, save on quit/disconnect, and save-before-unload for dirty columns. |
| Chunk loading/unloading while walking | M | Radius + 2 unload (world and renderer); memory bound not yet measured over a ten-minute live walk. |
| Time of day, weather, lightning | M | Persisted `Time`/`DayTime`/rain/thunder; day/night uses the renderer's `Environment` shader, with rain/lightning CLI overlays. |

### 10. Distribution and product

| Item | Size | Notes |
|---|---|---|
| Launcher v2: profiles, versions, news, update check | L | **Profiles + first-run setup landed.** Per-profile game directory and game window size; setup screen with live file count, progress bar and the failed-prepare log tail; unsigned build badge. No news feed or version switcher. |
| Signed auto-update | L | **Blocked on code signing.** The check is implemented (`NEPHELE_UPDATE_PUBLIC_KEY`, Ed25519 via `ring`) and is disabled when the key is empty, which it is today. Nothing downloads yet. |
| Crash reporter with opt-in upload | M | **Done.** Local `crashes/*.txt` by default, log tail included; upload only with an explicit launcher toggle and a configured `NEPHELE_CRASH_URL`. |
| Telemetry opt-in only, and never on by default | S | No telemetry exists; legal text says so. |
| Mod/plugin API for client modules | XL | Only if we want a third-party ecosystem. |
| Terms, privacy policy, non-affiliation disclaimer | S | **Done** in the launcher's Legal screen and the README. |
| Branding, installer, packaging | M | **Script-based installer done** (`scripts/package_windows.ps1` → `install.ps1`/`uninstall.ps1`), per-user, Start Menu entry, HKCU uninstall entry, clean updates that preserve data. No signed `.exe` installer. |

### 11. Testing and parity

| Item | Size | Notes |
|---|---|---|
| Java oracle parity harness (`Nephele-private/oracle`) | exists | Extend per feature: terrain, blocks, biome overrides. Stage-by-stage tracing landed 2026-11-05 (`trace.ps1` + `debug_stage_trace`). |
| Reference worlds for regression tests | exists | Seed 12345, structures off. |
| Performance benchmark on low-end hardware | M | Define the bar: 60 fps, Intel HD 4000-class. |
| Automated multiplayer smoke test against a real server | M | |

---

## Milestones

The M-numbers are retired — they collided (`M5` was both world saves and the
product; `M6` was both entities and performance). Lanes are named, and each one
reports a percentage rather than a status word. `AGENTS.md` §12 holds the branch
and worktree for each.

| Lane | Done | What is left |
|---|---:|---|
| `render` | 95% | shape audit leftovers; first-person hand depth-clear |
| `biomes` | 100% | nothing — every GenLayer stage diffs 0 against the oracle |
| `worldgen` | 15% | `provide_chunk`/`populate` split landed; caves, ores, flora, structures |
| `survival` | 90% | eating, potions, enchantments, XP orbs |
| `net` | 95% | resource packs, reconnect polish |
| `client-ui` | 20% | scoreboard rendering, chat colours, HUD modules, keybinds |
| `entities` | 60% | skins and second layer, equipment, frame rendering, particles |
| `audio` | 35% | music, volume settings, positional falloff tuning |
| `saves` | 90% | live memory walk; tile entities dropped on imported chunks |
| `perf` | 100% | Intel HD-class hardware still unverified |
| `product` | 70% | signed update and a clean-VM install pass; code signing is the external blocker |

**Overall: roughly 60% of a shippable PvP client.** What is missing is no longer
engine work — it is UI chrome, entity detail and content breadth.

---

## Backlog — small items worth writing down

**Ticks and movement**
- Fixed 20 Hz simulation with render interpolation
- Sprint resets when hunger drops or you take damage
- Sneak height change (`1.8` → `1.65`) and the camera drop
- Block outline exactly matches vanilla's box for non-cube blocks
- Correct `onGround` flag in position packets (servers use it for anti-cheat)
- Bobbing/view bobbing toggle

**Rendering**
- Vsync on/off plus an fps cap that actually caps
- FOV slider plus sprint/zoom modifiers
- GUI scale 1/2/3/auto
- Fullbright and brightness
- Named colour codes (`§`) and Unicode font

**UI**
- Pause menu completeness
- Connect screen remembers the last address
- Toggle sprint keybind defaults matching vanilla conventions
- Tooltips on hover for HUD elements

**Network**
- Keep-alive handling and timeout UI
- Compression threshold handling
- Chunk unload packets
- Entity spawn/despawn bookkeeping (some is in `entities.rs`)

**Misc**
- Localisation via `.lang`
- Screenshots (F2), fullscreen (F11), window icon
- Crash handler and in-game log viewer

---

## Open decisions

1. **Renderer backend.** wgpu (Vulkan/DX12) is modern and clean, but old Intel
   iGPUs and stale drivers break on it. OpenGL is what every competitor uses and
   what old laptops actually support. Options: keep wgpu and accept the
   compatibility risk; keep wgpu and add a GL/GLES fallback; switch to GL
   entirely. **Decide by benchmarking on the oldest hardware you care about.**
   Preference order for the product: GL first, wgpu where available.
2. **Auth scope.** Device-code flow only, or full browser flow too? Device code
   is far simpler and works everywhere; browser flow is friendlier for children.
3. **Client modules.** Built-in only, or a plugin API for third parties? A plugin
   API is an XL investment that only pays off with an ecosystem.
4. **Multi-version.** 1.8.9 only, or 1.7.10/1.12.2 later? Every extra version
   doubles protocol and rendering-shape work.
5. **Telemetry.** Ship at all? If yes, opt-in, anonymised, and documented.
