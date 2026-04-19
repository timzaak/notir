# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Development Commands

### Rust Backend
```bash
cargo build                  # Debug build
cargo build --release        # Release build
cargo test                   # Run all tests
cargo test test_single_shot  # Run a single test by name
cargo fmt --all -- --check   # Check formatting
cargo clippy --all-targets --all-features -- -D warnings  # Lint
cargo run                    # Run server (default port 5800)
cargo run -- --port 8698     # Run on custom port
```

### Frontend (in `frontend/`)
```bash
cd frontend
npm install
npm run dev        # Dev server with proxy to backend at localhost:5800
npm run build      # TypeScript check + Vite production build → dist/
npm run lint       # ESLint
```

### Docker
```bash
docker build .     # Multi-stage: frontend build → Rust build → minimal runtime
```

The Dockerfile builds the frontend first (`npm run build`), copies the output into `static/`, then compiles the Rust binary which embeds those static files via `rust-embed`.

## Architecture

**Notir** is a lightweight WebSocket message server. A Rust backend (Salvo + Tokio) handles WebSocket connections and HTTP publish endpoints; a React frontend (Vite + TypeScript + Tailwind + CodeMirror) provides a web UI for testing.

### Backend (`src/`)

- **`main.rs`** — Entry point. Parses CLI args (port), sets up Salvo routing, serves embedded static files with gzip compression. Routes:
  - `WS /single/sub?id=<uid>` — Subscribe to point-to-point messages
  - `POST /single/pub?id=<uid>&mode=<shot|ping_pong>` — Send to a user
  - `WS /broad/sub?id=<uid>` — Subscribe to broadcast channel
  - `POST /broad/pub?id=<uid>` — Broadcast to all subscribers of a channel
  - `GET /health`, `GET /version`, `GET /connections?id=<uid>`

- **`single.rs`** — Point-to-point messaging with two modes:
  - **Shot**: one-way fire-and-forget delivery
  - **PingPong**: sends message and waits up to 5s for client response via `oneshot` channel
  - Global state: `ONLINE_USERS` (DashMap<user_id, DashMap<conn_id, mpsc sender>>), `CALLBACK_CHANNELS` (DashMap<user_id, VecDeque<(id, oneshot sender)>>)
  - Each connection gets a nanoid, a 30s ping heartbeat, and auto-cleanup on disconnect

- **`broadcast.rs`** — One-to-many messaging:
  - Subscribers connect via WebSocket, only receive messages (client-sent messages ignored except pong)
  - Global state: `BROADCAST_USERS` (RwLock<HashMap<broadcast_id, Vec<Connection>>>)
  - Uses atomic counter for connection IDs, 30s ping heartbeat, failed-connection cleanup on publish

- **`tests.rs`** — Unit tests for both modules (mode deserialization, connection lifecycle, message delivery, timeout handling, concurrent access)

### Frontend (`frontend/src/`)

- **`WebSocketManager.ts`** — WebSocket connection management with reconnection logic
- **`WebSocketConfig.tsx`** — Configuration UI for connection modes
- **`CodeEditor.tsx`** — CodeMirror editor for composing messages
- Vite dev server proxies `/single/sub`, `/broad/sub`, and `/version` to `localhost:5800`

### Static File Serving

The `static/` directory is embedded into the Rust binary at compile time via `rust-embed`. For local development, run the backend and frontend dev servers separately; for production, build the frontend into `static/` first (handled by Dockerfile).

## CI

GitHub Actions (`.github/workflows/ci.yml`) runs on push to main: `cargo fmt --check`, `cargo clippy`, `cargo build`, `cargo test`. Rust toolchain: 1.89. Edition 2024.
