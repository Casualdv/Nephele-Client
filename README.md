# Nephele

A native, from-scratch client for Minecraft: Java Edition 1.8.9 (protocol 47), written in Rust.

> **Not an official Minecraft product.** Not approved by, endorsed by, or
> associated with Mojang or Microsoft. "Minecraft" is a trademark of Mojang
> Synergies AB.
>
> Nephele contains no Minecraft code, assets or data. Everything it loads is
> built on your machine from the copy of the game you already own. It never
> downloads game files and never ships them.

## Status

Early, but a real client: it renders the world with vanilla's own textures,
talks protocol 47, and joins offline-mode servers.

**Working**

- Protocol 47: handshake, status ping, login and play, with zlib compression
- World: chunk decoding, block states, lighting, raycasting, and vanilla terrain
  generation (GenLayer biomes, density height field, per-biome surface), used by
  both singleplayer and the offline viewer
- Rendering: wgpu 30 with worker-threaded chunk meshing, vanilla block models,
  `.mcmeta` animations, mipmapped textures, procedural sky and per-biome fog,
  frustum + height-field occlusion culling and a per-frame upload budget
  and textures, the GUI atlas, the title-screen panorama and splashes
- Block shapes: slabs, stairs, fence posts, panes, doors and torches render with shape-aware geometry
- Movement and interaction: vanilla travel physics, sprinting, mining, block
  placement, creative item palette, chat
- Client HUD: toggleable, draggable modules (FPS, ping, CPS, keystrokes,
  coordinates, speed), a scoreboard sidebar and coloured chat, with settings
  saved to `settings.json`
- Options tree: Video Settings, rebindable controls with conflict handling,
  volumes, chat settings, language and resource packs - all persisted and applied
  without a restart (where the renderer has a hook)
- Players: entity tracking, skins and skin loading, classic/slim humanoid
  rendering with the second layer, held items and armour, crouch and arm swing,
  team-coloured nametags with tab-list ping, and framed maps with real pictures
- Particles: block debris, note blocks and the `S2A` sprite kinds (smoke,
  flame, crit, portal, splash, heart, block/item cracks) drawn on the same
  texture array
- Items: server drops render as spinning entities; local survival adds mined
  block items to inventory
- First person: a skin-textured arm holding the selected block, with a swing while mining
- Inventory: survival panel with 2x2 crafting, armour, storage and hotbar;
  supported tool, armour and food icons
- Crafting: shared recipes in the survival 2×2 inventory and a usable placed-table 3×3 grid
- Chat: on-screen log and input, with client-side `/gamemode`, `/tp`, `/setblock`, `/give`
- UI: title, multiplayer (live ping, MOTD and version), direct connect, pause, settings
  and creative screens; HUD, hotbar, hearts
- Audio: block break and place, footsteps and the player hurt sound, from the
  vanilla sound events, with distance attenuation and stereo panning

**Not implemented yet**

- Microsoft sign-in and the protocol's encryption handshake, so **online-mode
  servers cannot be joined** — Nephele only speaks to offline-mode servers
- Mob sounds, mob behaviour, survival gameplay, world saving and loading

## Requirements

- Rust 1.82 or newer (`rust-toolchain.toml` tracks stable with `rustfmt` and `clippy`)
- A GPU with Vulkan, DX12 or Metal, driven through wgpu 30
- **Your own copy of Minecraft: Java Edition 1.8.9**, downloaded once by your
  launcher. This is required, and Nephele cannot obtain it for you.

## Build and run

```sh
cargo run -p nephele-prepare      # one-time: build resources from your own copy
cargo run -p nephele -- play      # title screen
```

Joining a server:

```sh
cargo run -p nephele -- play play.example.net --username Steve
cargo run -p nephele -- status play.example.net      # MOTD, version, latency
```

Trying it without any server, using the built-in mock world:

```sh
cargo run -p nephele -- mock-server --bots            # terminal 1, port 25565
cargo run -p nephele -- play 127.0.0.1:25565          # terminal 2
```

`cargo run -p nephele -- view` renders a generated world with no networking at
all, which is the quickest way to see the renderer work.

### The launcher

```sh
cargo run -p nephele-launcher
```

A window with Play, a demo-world button and the resource status. It runs
`nephele-prepare` by itself the first time resources are needed, and launches
the game as a **child process**, so a crash in the client leaves the launcher
open with a message instead of taking the whole app down with it.

### Launching without a console

Double-click **`Launch Nephele.cmd`** in the repository root. It builds the three
binaries into `target\release` and starts the launcher from there, next to the
`nephele.exe` and `nephele-prepare.exe` it spawns. Because it builds first, the
shortcut always runs the code you just edited; an up-to-date build costs about a
second, since cargo only recompiles what changed.

```bat
Launch Nephele.cmd            :: release build (recommended)
Launch Nephele.cmd debug      :: debug build: faster to build, slower to play
```

To produce a folder you can zip and hand to someone else, run:

```powershell
scripts\build_launcher.ps1    # builds all three and fills dist\
```

`dist\` then holds `nephele-launcher.exe`, `nephele.exe` and
`nephele-prepare.exe` (about 16 MB total) plus the licence. Those three must
stay together: the launcher starts the other two by name from its own folder.
The binaries are not code-signed yet, so Windows shows a SmartScreen warning on
first run.

### Preparing the resources

`nephele-prepare` reads the 1.8.9 client jar you already have, checks it against
the SHA-1 pinned in `research-lock.json`, and extracts the textures, models and
language files the client reads into `.local/prepared/1.8.9/` (gitignored).

It finds your install automatically. Otherwise point it at one:

```sh
cargo run -p nephele-prepare -- --minecraft-dir "C:\Users\you\AppData\Roaming\.minecraft"
cargo run -p nephele-prepare -- --jar /path/to/1.8.9.jar
cargo run -p nephele-prepare -- --verify-only        # rehash an existing preparation
```

A standard game directory, a Prism/MultiMC instance and a launcher root are all
understood. Nothing is downloaded: if the jar is missing, start 1.8.9 once in
your launcher and run it again.

## FAQ

**Does Nephele ship Minecraft?**
No. There are no Mojang binaries, assets or extracted tables in this
repository, and `scripts/check_public_tree.ps1` fails the build if any appear.
Textures, models and language files are read out of *your* 1.8.9 jar, on your
machine, by `nephele-prepare`.

**Why 1.8.9?**
Protocol 47. It is the version the PvP servers this client is built for still
run, and its movement and combat numbers are the ones the physics is pinned to.

**Do I need Java installed?**
No. The jar is only read as a zip.

**Do I need to prepare resources every time I build?**
No. `nephele-prepare` is one-time per asset set; the launcher re-runs it only
when the prepared folder is missing or out of date.

**The launcher says "no 1.8.9 resources".**
It could not find your install. Point it at one explicitly:
`cargo run -p nephele-prepare -- --minecraft-dir "C:\Users\you\AppData\Roaming\.minecraft"`.
Nothing is downloaded; start 1.8.9 once in your launcher first.

**Windows says the app is unsafe.**
The binaries are unsigned. There is no code-signing certificate on the project
yet, so SmartScreen warns on every fresh download. Build it yourself from source
if you would rather not trust a binary.

**Online mode says 403 (Invalid session).**
Cached Microsoft tokens last about 24 hours, and the client does not yet refresh
them automatically. Run `cargo run -p nephele -- auth` and join again. Automatic
refresh is a known gap, not a permissions problem.

**Where are my settings, servers and worlds?**
`%APPDATA%\Nephele` (or `$XDG_DATA_HOME/nephele`, or `~/.local/share/nephele`):
`settings.json`, `servers.json`, `auth.json` and `saves\`. `NEPHELE_DATA_DIR`
overrides the location — `scripts/run_client.ps1 -Session <name>` uses it so two
clients never share one.

**Grass looks plain green and some plants look black or white.**
Two known issues, both tracked. Grass tint needs the per-biome colour table
(`BiomeColorizer`), which is not on `main` yet; until it lands the tint is the
plains colour. Plants are meshed correctly as crossed quads but a texture name
that does not resolve to a packed atlas tile draws as the missing tile, which
reads as a black or white X on the ground.

**Can I play on a server that uses a resource pack?**
Not yet. The client answers the pack offer but cannot apply the pack, so
server-customised textures fall back to vanilla.

**How do I report a bug?**
Include the `NEPHELE_DEBUG=1` line (fps, position, chunk count), the seed for
world-generation bugs, and whether it happens on a server or in singleplayer.
Build and test gates are listed under *Development* below; a report that says
which of them fail is worth ten that say "it broke".

## What is and is not in this repository

| In the repository | Not in the repository |
| --- | --- |
| Nephele's own Rust source | The Minecraft client jar, ever |
| `research-lock.json` — the pinned jar checksum | Extracted textures, models, sounds |
| The public-release gate | Asset indexes, libraries, worlds |

Two things enforce that:

- `.gitignore` denies jars, class files, executables, asset trees and `.local/`.
- `scripts/check_public_tree.ps1` scans **what a clone would publish** and fails
  on Mojang artifacts, on zip magic in a renamed file, and on compile-time
  asset embedding (`include_bytes!`/`include_str!` of game files). CI runs it,
  so a mistake cannot reach `main`.

```sh
pwsh scripts/check_public_tree.ps1
```

## Layout

| Crate | Responsibility |
| --- | --- |
| `nephele-protocol` | Packets, framing, compression, NBT, addresses, encryption primitives |
| `nephele-world` | Chunk storage, chunk encode/decode, block states, terrain generation, raycasting |
| `nephele-client` | Connection state machine, physics, mining, entity and skin tracking |
| `nephele-assets` | Texture arrays, block models, GUI atlas, panorama, skin atlas |
| `nephele-render` | wgpu renderer: terrain, entities, UI, lines, camera |
| `nephele-mock` | A fake 1.8.9 server with a deterministic world, for tests and demos |
| `nephele-prepare` | Verified resource preparation from your own copy of the game |
| `nephele` | The binary: `play`, `view`, `status`, `login`, `join`, `mock-server` |

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pwsh scripts/check_public_tree.ps1
```

All of these run in CI on every push and pull request.

Most tests are self-contained: the world generator, the protocol codecs, the
block tables and the UI geometry need no game files. The handful that read
vanilla assets (block models, the panorama, the splash text) skip with a note
when the assets have not been prepared, so `cargo test` works on a fresh clone.
Run `nephele-prepare` and they run for real.
