{ withSystem, ... }:
{
  flake.nixosModules.spektra =
    {
      config,
      lib,
      pkgs,
      ...
    }:
    let
      cfg = config.services.spektra;

      inherit (lib)
        mkOption
        types
        mkIf
        optional
        ;
    in
    {
      options.services.spektra = {
        enable = lib.mkEnableOption "the Spektra central server";

        package = mkOption {
          type = types.package;
          default = withSystem pkgs.stdenv.hostPlatform.system ({ config, ... }: config.packages.server);
          defaultText = lib.literalExpression "the `server` package for the host's own system";
          description = "The server package to run.";
        };

        staticDir = mkOption {
          type = types.path;
          default = withSystem pkgs.stdenv.hostPlatform.system ({ config, ... }: config.packages.static);
          defaultText = lib.literalExpression "the `static` package for the host's own system";
          description = ''
            Directory served at `/static`: the built stylesheet, Leaflet and
            HTMX. Separate from `package` so the web client can be rebuilt
            without relinking the server.
          '';
        };

        host = mkOption {
          type = types.str;
          default = "127.0.0.1";
          description = "Address the REST API binds to.";
        };

        port = mkOption {
          type = types.port;
          default = 8080;
          description = "Port for the REST API, the web client and Swagger UI.";
        };

        grpcHost = mkOption {
          type = types.str;
          default = "0.0.0.0";
          description = "Address the gRPC ingest listener binds to.";
        };

        grpcPort = mkOption {
          type = types.port;
          default = 50051;
          description = "Port for gRPC ingest from the node fleet.";
        };

        logLevel = mkOption {
          type = types.str;
          default = "server=info";
          example = "server=debug,tower_http=debug";
          description = "Value of `RUST_LOG` for the service.";
        };

        ntfyUrl = mkOption {
          type = types.nullOr types.str;
          default = null;
          example = "https://ntfy.sh";
          description = ''
            Value of `SPEKTRA_NTFY_URL`. The topic goes in `environmentFile`:
            on a public server anyone who knows it can subscribe.
          '';
        };

        database = {
          createLocally = mkOption {
            type = types.bool;
            default = true;
            description = ''
              Provision PostgreSQL on this host and connect over the local unix
              socket with peer authentication. Set to false to point the server
              at an external database through `environmentFile`.
            '';
          };

          name = mkOption {
            type = types.str;
            default = "spektra";
            description = ''
              Database name, also used as the PostgreSQL role name so that
              `ensureDBOwnership` applies.
            '';
          };
        };

        environmentFile = mkOption {
          type = types.nullOr types.path;
          default = null;
          description = ''
            Environment file read at start, for anything that must not be
            world-readable in the store:

            - `DATABASE_URL`, when the database is not created locally.
            - `SPEKTRA_ADMIN_EMAIL` and `SPEKTRA_ADMIN_PASSWORD`, which create
              the administrator on the first start against an empty `users`
              table and are ignored on every start after that.
            - `SPEKTRA_NTFY_TOPIC` and the optional `SPEKTRA_NTFY_TOKEN`.
              Without a topic and `ntfyUrl`, alarms reach the live channel
              but no phone.
          '';
        };
      };

      config = mkIf cfg.enable {
        assertions = [
          {
            assertion = cfg.database.createLocally || cfg.environmentFile != null;
            message = ''
              services.spektra: with database.createLocally = false you must set
              environmentFile to something that provides DATABASE_URL.
            '';
          }
        ];

        services.postgresql = mkIf cfg.database.createLocally {
          enable = true;
          package = pkgs.postgresql_18;

          settings = {
            max_wal_size = "8GB";
            shared_buffers = "2GB";
            effective_cache_size = "6GB";
          };

          ensureDatabases = [ cfg.database.name ];
          ensureUsers = [
            {
              name = cfg.database.name;
              ensureDBOwnership = true;
            }
          ];
        };

        systemd.services.spektra = {
          description = "Spektra central server: gRPC ingest and web API";
          wantedBy = [ "multi-user.target" ];
          wants = [ "network-online.target" ];
          after = [
            "network-online.target"
          ]
          ++ optional cfg.database.createLocally "postgresql.target";
          requires = optional cfg.database.createLocally "postgresql.target";

          environment = {
            BIND_ADDR = "${cfg.host}:${toString cfg.port}";
            GRPC_BIND_ADDR = "${cfg.grpcHost}:${toString cfg.grpcPort}";
            RUST_LOG = cfg.logLevel;
            SPEKTRA_STATIC_DIR = toString cfg.staticDir;
          }
          // lib.optionalAttrs cfg.database.createLocally {
            DATABASE_URL = "postgres:///${cfg.database.name}?host=/run/postgresql";
          }
          // lib.optionalAttrs (cfg.ntfyUrl != null) {
            SPEKTRA_NTFY_URL = cfg.ntfyUrl;
          };

          serviceConfig = {
            ExecStart = lib.getExe' cfg.package "server";
            Restart = "on-failure";
            RestartSec = 5;

            DynamicUser = true;
            User = cfg.database.name;

            EnvironmentFile = optional (cfg.environmentFile != null) cfg.environmentFile;

            AmbientCapabilities = [ ];
            CapabilityBoundingSet = [ ];
            LockPersonality = true;
            MemoryDenyWriteExecute = true;
            NoNewPrivileges = true;
            PrivateDevices = true;
            PrivateTmp = true;
            ProcSubset = "pid";
            ProtectClock = true;
            ProtectControlGroups = true;
            ProtectHome = true;
            ProtectHostname = true;
            ProtectKernelLogs = true;
            ProtectKernelModules = true;
            ProtectKernelTunables = true;
            ProtectProc = "invisible";
            ProtectSystem = "strict";
            RemoveIPC = true;
            RestrictAddressFamilies = [
              "AF_INET"
              "AF_INET6"
              "AF_UNIX"
            ];

            RestrictNamespaces = true;
            RestrictRealtime = true;
            RestrictSUIDSGID = true;
            SystemCallArchitectures = "native";
            SystemCallFilter = [
              "@system-service"
              "~@privileged"
              "~@resources"
            ];

            UMask = "0077";
          };
        };
      };
    };
}
