## dlob-server-rs

`dlob-server-rs` is a Rust backend service that builds on the `drift-rs` SDK to provide a high‑performance,
off‑chain view of the Drift V2 decentralized limit order book (DLOB) on Solana.

It connects to Drift markets via Solana RPC, maintains an in‑memory DLOB using `drift-rs`, and publishes
real‑time order book snapshots (L2) and metadata to Redis so that downstream services (WebSocket/HTTP gateways,
trading bots, analytics pipelines, etc.) can consume them with minimal latency.

For more information about the underlying SDK and protocol, see the main `drift-rs` README in this repository
and the official Drift documentation.

## Features

- Real‑time DLOB snapshots built from on‑chain Drift user accounts
- Support for multiple market types via `MarketId` (perp and spot)
- Periodic publishing of L2 order book data into Redis (pub/sub channels + cached keys)
- Health checking logic that can mark the process unhealthy if oracle slots diverge too far from DLOB slots
- Environment‑driven configuration for RPC, WebSocket, Redis, and Drift environment
- Built on `tokio` + `axum`, ready to be extended with HTTP/WS APIs and Prometheus metrics

## Architecture Overview

At a high level:

1. **Configuration** is loaded from environment variables via `Config::from_env` (`src/config.rs`).
2. A **`DriftClient`** is created using the provided Solana RPC endpoint and Drift context.
3. The client **syncs user accounts** and builds an in‑memory **`DLOB`** via `DLOBBuilder`.
4. A **`DLOBSubscriber`** is created, which:
   - Wraps the `DLOB` and `DriftClient`
   - Tracks configured `WsMarketArgs` (markets + depth, etc.)
   - Periodically produces L2 snapshots and publishes them to Redis channels/keys.
5. A background **Tokio task** runs an interval loop that calls `get_l2_and_send_msg` for each market,
   which:
   - Fetches an up‑to‑date L2 snapshot from the DLOB
   - Performs basic oracle/slot health checks
   - Computes top‑of‑book, mid, spread (absolute and pct)
   - Publishes a JSON payload to Redis.

You can layer your own HTTP/WS API or metrics on top of this core loop using the existing `axum`,
`prometheus`, and health modules under `src/core`.

## Environment Variables

Configuration is primarily driven by environment variables loaded in `src/config.rs` and
`src/core/health.rs`:

| Variable               | Description                                                              | Example                             |
| ---------------------- | ------------------------------------------------------------------------ | ----------------------------------- |
| `RPC_URL`              | Solana HTTP RPC endpoint used by the Drift client.                      | `https://your-private-rpc.com`      |
| `WS_URL`               | Solana WebSocket endpoint (reserved for future WS‑based subscriptions). | `wss://your-private-rpc.com`        |
| `REDIS_URL`            | Redis connection URL for publishing DLOB snapshots.                     | `redis://127.0.0.1/`                |
| `DRIFT_ENV`            | Drift environment, e.g. `mainnet` or default `devnet`.                  | `mainnet`                           |
| `MAX_SLOT_STALENESS_MS`| Max allowed time without slot updates before considered unhealthy.      | `5000`                              |
| `MIN_SLOT_RATE`        | Minimum acceptable slot update rate (slots/sec) for health checks.      | `0.03`                              |

> Any missing required variables (`RPC_URL`, `WS_URL`) will cause the process to fail at startup.

## Running the Server

1. **Install Rust toolchains** as described in the repository root `README.md` (x86_64 target required).
2. From the repo root, build the project:

```bash
cargo build -p dlob-server-rs
```

3. Create a `.env` file next to `dlob-server-rs/Cargo.toml` (or export env vars) with at least:

```bash
RPC_URL=https://your-private-rpc.com
WS_URL=wss://your-private-rpc.com
REDIS_URL=redis://127.0.0.1/
DRIFT_ENV=mainnet
```

4. Run the server:

```bash
cargo run -p dlob-server-rs
```

On startup, the server will:

- Initialize logging/tracing
- Build a `DriftClient` for the configured `DRIFT_ENV`
- Sync Drift user accounts into memory
- Construct a `DLOB` for the configured markets
- Start a background task that periodically publishes L2 snapshots to Redis.

Use a separate WebSocket/HTTP process (or extend this crate with `axum` routes) to read from Redis and
serve DLOB data to external clients.

## Redis Publishing Model

The `DLOBSubscriber` publishes L2 data to Redis using:

- **Pub/Sub channel**: `orderbook_{market_type}_{market_index}`
- **Last update key**: `last_update_orderbook_{market_type}_{market_index}`

Each message is a JSON object with fields like:

- `market_name`, `market_type`, `market_index`
- `bids`, `asks` (arrays of `{ price, size }`)
- `slot`, `ts`
- `oracle`, `mark_price`
- `best_bid_price`, `best_ask_price`
- `spread_pct`, `spread_quote`

This mirrors the semantics of the JavaScript DLOB server while leveraging Rust and `drift-rs`
under the hood.

## Health and Liveness

The `core::health` module tracks:

- Last seen slot and timestamp
- Slot update rate vs. `MIN_SLOT_RATE`
- Maximum slot staleness vs. `MAX_SLOT_STALENESS_MS`

Additionally, `DLOBSubscriber::get_l2_and_send_msg` compares the DLOB slot to the oracle slot and
can set the global health status to `Restart` if the divergence exceeds a kill‑switch threshold.
You can wire this into Kubernetes liveness/readiness probes or your own process supervisor.

## Extending the Server

The crate is structured to be easily extended:

- `src/api/` – add HTTP/WS handlers with `axum`.
- `src/core/` – health checks, metrics, and middleware.
- `src/athena/` – integration points for external analytics/storage backends.
- `src/publishers/` – additional sinks (e.g. Kafka, NATS, REST push).
- `src/utils/` – shared helpers.

If you are already familiar with the TypeScript `dlob-server` repository, you can think of
`dlob-server-rs` as the Rust counterpart that focuses on the core DLOB building, health checking,
and Redis publishing, leaving the surrounding HTTP/WebSocket surface up to you or your deployment.
