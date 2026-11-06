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
- World: chunk decoding, block states, lighting, raycasting, terrain generation
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
- Players: entity tracking, skins and skin loading, humanoid rendering
- Items: server drops render as spinning entities; local survival adds mined
  block items to inventory
- First person: a skin-textured arm holding the selected block, with a swing while mining
- Inventory: survival panel with 2x2 crafting, armour, storage and hotbar;
  supported tool, armour and food icons
- Crafting: shared recipes in the survival 2×2 inventory and a usable placed-table 3×3 grid
- Chat: on-screen log and input, with client-side `/gamemode`, `/tp`, `/setblock`, `/give`
- UI: title, multiplayer (live ping, MOTD and version), direct connect, pause, settings
  and creative screens; HUD, hotbar, hearts

**Not implemented yet**

- Microsoft sign-in and the protocol's encryption handshake, so **online-mode
  servers cannot be joined** — Nephele only speaks to offline-mode servers
- Sound, mob behaviour, survival gameplay, world saving and loading

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
