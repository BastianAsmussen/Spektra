[English](README.md) | **Dansk**

# Spektra

Distribueret overvågning af signalkvalitet på FM/DAB. SDR-noder rapporterer
aggregerede målinger til en central server over en versioneret gRPC-protokol.
Serveren beregner en baseline per node og kanal, alarmerer ved afvigelse, og en
HTMX-webklient visualiserer flåden.

## Workspace

- `protocol/`: gRPC-protokollen (`proto/v1/*.proto`) og dens genererede Rust-typer.
- `server/`: Axum REST-API, tonic-ingest, PostgreSQL-lagring, Swagger-dokumentation.
- `node-agent/`: SDR-nodeagenten.
- `emulator/`: tonic-klient der registrerer falske noder og sender rapporter. `emulator/many.sh` starter en flåde.

## Udvikling

```sh
nix develop
docker compose -f compose.test.yaml up -d

DATABASE_URL=postgres://spektra_test:test_password@localhost:5432/spektra_test cargo run -p server

cargo test --release # Med testdatabasen oppe.
cargo build --release
```

REST på `:8080` (Swagger på `/swagger-ui/`), gRPC-ingest på `:50051`.
Sæt `BIND_ADDR` til den ønskede IP og port. Standard er `0.0.0.0:8080`.

Kør flåde-emulatoren:

```sh
cargo build --release -p emulator
./emulator/many.sh -n 100
```
