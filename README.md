**English** | [Dansk](README.da.md)

# Spektra

Distributed FM/DAB signal-quality monitoring. SDR nodes report aggregated
measurements to a central server over a versioned gRPC protocol. The server
baselines per node and channel, alarms on deviation, and an HTMX web client
visualizes the fleet.

## Workspace

- `protocol/`: The gRPC protocol (`proto/v1/*.proto`) and its generated Rust types.
- `server/`: Axum REST API, tonic ingest, PostgreSQL storage, Swagger docs.
- `node-agent/`: The SDR node agent.
- `emulator/`: tonic client that registers fake nodes and sends reports. `emulator/many.sh` spawns a fleet.

## Development

```sh
nix develop
docker compose -f compose.test.yaml up -d

DATABASE_URL=postgres://spektra_test:test_password@localhost:5432/spektra_test cargo run -p server

cargo test --release # With the test database up.
cargo build --release
```

REST on `:8080` (Swagger at `/swagger-ui/`), gRPC ingest on `:50051`.
Set `BIND_ADDR` to the desired IP and port. Defaults to `0.0.0.0:8080`.

Run the fleet emulator:

```sh
cargo build --release -p emulator
./emulator/many.sh -n 100
```
