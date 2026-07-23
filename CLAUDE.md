# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Hoot! is a self-hosted, local-network quiz game (Kahoot-style) for classrooms, parties, and small in-person events. A shared host screen displays questions; players join from their phones as private answer controllers. Single Rust/Axum backend process serves the built Svelte frontend, a WebSocket API, and local media, all from one port (default 8080).

## Commands

Run from the repo root unless noted.

```bash
npm install && npm --prefix frontend install   # first-time setup
npm run build                                   # build frontend (vite) + backend (cargo --release)
npm start                                       # run the built server: cargo run --release -p hoot-server
npm run dev                                     # concurrently run backend (cargo run) + Vite dev server (port 5173, proxies /api and /media to Axum on 8080)
npm test                                        # cargo test --workspace && frontend vitest (--run)
npm run check                                   # cargo fmt --check && cargo clippy --workspace --all-targets -D warnings && svelte-check
```

Single-crate / targeted commands:

```bash
cargo test -p hoot-server                       # backend tests only
cargo test -p hoot-server <test_name>            # a single backend test (tests live in #[cfg(test)] mods inside main.rs/engine.rs/content.rs)
npm --prefix frontend test                       # frontend tests only (vitest)
npm --prefix frontend run check                  # svelte-check only
```

Runtime configuration (flags or env vars — see `README.md` for full details):

```bash
npm start -- --games content/games.json --bind 0.0.0.0:8080
# HOOT_GAMES, HOOT_BIND, HOOT_FRONTEND, HOOT_PUBLIC_URL
```

Set `HOOT_PUBLIC_URL` whenever automatic LAN-interface detection picks the wrong address (e.g. when running inside WSL2 — the server auto-detects this case and prints/shows a warning).

## Architecture

### Backend (`backend/src/`) — one process, one game at a time

- **`model.rs`** — plain data types with no logic: `Game`/`Question`/`Choice` (parsed from `content/games.json`), the `Phase` enum (the game's state machine: `Selection → Lobby → Reading → Answering → Reveal → Leaderboard → ... → Podium → FinalLeaderboard`), `Player`, `Submission`, and `GameState` (the single source of truth — revision counter, host token hash, active game, phase, players, submissions, advertised join URL, network warning).
- **`engine.rs`** — `Engine` wraps `GameState` + the loaded `Catalog` and holds all game logic: joining players, selecting/starting a game, advancing phases, per-question scoring (speed + double-points), free-text answer matching (Unicode-normalized with typo tolerance), and building the JSON snapshots (`host_snapshot`/`player_snapshot`) sent to clients. **Player snapshots are redacted** — they never reveal other players' answers or the correct answer before reveal.
- **`content.rs`** — `Catalog::load` parses and validates `content/games.json` (schema, image existence/size/type under `content/media/`) into `Game`s; reload happens via a host command without restarting the server.
- **`main.rs`** — Axum app: HTTP routes (`/api/health`, `/api/players/join`, `/api/host/claim`) plus a single `/api/ws` WebSocket endpoint used by *both* hosts and players (role distinguished by an `authenticate` message with a role + opaque token, matched against a hashed token in `GameState`). No CORS layer exists — the frontend is served from the same origin, so none is needed. A background `spawn_clock` task ticks every 100ms to auto-advance time-based phases (reading/answering/podium timeouts) via `Engine::tick`. `detect_join_urls` auto-discovers LAN-reachable addresses via `local-ip-address`; `is_wsl2()`/`wsl2_network_warning()` detect the common WSL2-NAT trap where auto-detected addresses aren't reachable from other devices.
- State lives entirely in server memory (`Arc<Mutex<Engine>>`). There is no database and no persistence across restarts — restarting the server intentionally wipes players, scores, and tokens. Session tokens are opaque and stored client-side in `localStorage`, so browser refreshes/reconnects survive as long as the server process is still running.
- Any state change calls `Engine::bump()` (increments `revision`) and broadcasts via a `tokio::sync::broadcast` channel; every connected WebSocket client then receives a fresh full-state snapshot (not a diff).

### Frontend (`frontend/src/`) — Svelte 5, no router, no framework state store

- **`App.svelte`** picks `Host.svelte` or `Player.svelte` based on `location.pathname === '/host'`; that's the entire routing layer.
- **`Host.svelte`** / **`Player.svelte`** are large single-file views that switch their entire rendered section based on `state.phase.name` (mirroring the backend `Phase` enum) inside one big `{#if phaseIs(...)}` chain. There is no client-side game-state store — `state` is a plain top-level `let` reassigned wholesale on every WebSocket snapshot.
- **`lib/connection.ts`** owns the single WebSocket connection per view: sends `{type: 'authenticate', role, token}` on open, exponential-backoff auto-reconnects on close, and queues (single-slot, most-recent-wins) any command sent while disconnected/unauthenticated, flushing it once the next snapshot confirms auth succeeded.
- **`lib/types.ts`** mirrors the backend's JSON snapshot shape (`HostState`/`PlayerState`/`Phase`/etc.) — keep these in sync by hand when `engine.rs`'s `host_snapshot`/`player_snapshot` JSON shape changes; there is no shared schema/codegen between Rust and TypeScript.
- **`lib/Countdown.svelte`** renders remaining time from a server-provided `deadline_ms` plus a client/server clock offset (`serverOffsetMs`, computed once per connection from `state.serverTimeMs` — not recomputed on every snapshot, to avoid visible jitter).
- **Gotcha:** don't drive async side effects (e.g. QR code generation) from a Svelte `$:` reactive statement that also affects the same component whose `{:else if phaseIs(...)}` block needs to re-render — combining the two caused the host screen's phase transitions to silently stop updating in production even though the WebSocket snapshots were received correctly. Prefer calling such side effects imperatively from the state-update callback instead.
- No CSS framework — a single `frontend/src/styles.css` with a `:root` custom-property token palette (ink/cream/pastel host+player backgrounds, gold/purple accents, red/blue/amber/green for answer tiles and controller buttons, success/danger semantic pairs). Keep new colors on tokens rather than introducing bare hex values.

### Content authoring

`content/games.json` is hand-edited (schema documented in `README.md`); images referenced by filename live in `content/media/`. The host can reload content live via a WebSocket command without restarting the server or losing the current session.
