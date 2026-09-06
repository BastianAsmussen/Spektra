{ withSystem, ... }:
{
  flake.nixosModules.nodeAgent =
    {
      config,
      lib,
      pkgs,
      ...
    }:
    let
      cfg = config.services.spektra-node-agent;

      inherit (lib) mkOption types;
    in
    {
      options.services.spektra-node-agent = {
        enable = lib.mkEnableOption "the Spektra SDR node agent";

        package = mkOption {
          type = types.package;
          default = withSystem pkgs.stdenv.hostPlatform.system ({ config, ... }: config.packages.node-agent);
          defaultText = lib.literalExpression "the `node-agent` package for the host's own system";
          description = "The node agent package to run.";
        };

        server = mkOption {
          type = types.str;
          example = "http://spektra.example.org:50051";
          description = "gRPC endpoint of the central server.";
        };

        logLevel = mkOption {
          type = types.str;
          default = "node_agent=info";
          description = "Value of `RUST_LOG` for the agent.";
        };
      };

      config = lib.mkIf cfg.enable {
        hardware.rtl-sdr.enable = true;

        services.chrony.enable = lib.mkDefault true;

        users = {
          groups.spektra-node = { };
          users.spektra-node = {
            isSystemUser = true;
            group = "spektra-node";
            description = "Spektra node agent";
          };
        };

        systemd.services.spektra-node-agent = {
          description = "Spektra node agent: SDR sampling, DSP and reporting";
          wantedBy = [ "multi-user.target" ];
          wants = [ "network-online.target" ];
          after = [ "network-online.target" ];

          environment = {
            SPEKTRA_SERVER = cfg.server;
            SPEKTRA_STATE_DIR = "/var/lib/spektra-node-agent";
            RUST_LOG = cfg.logLevel;

            SOAPY_SDR_PLUGIN_PATH = "${pkgs.soapysdr-with-plugins}/lib/SoapySDR/modules0.8";
          };

          startLimitBurst = 5;
          startLimitIntervalSec = 300;
          serviceConfig = {
            ExecStart = lib.getExe cfg.package;
            Restart = "on-failure";
            RestartSec = 10;
            User = "spektra-node";
            Group = "spektra-node";
            SupplementaryGroups = [ "plugdev" ];

            StateDirectory = "spektra-node-agent";
            StateDirectoryMode = "0700";

            LockPersonality = true;
            MemoryDenyWriteExecute = true;
            NoNewPrivileges = true;
            PrivateTmp = true;
            ProtectControlGroups = true;
            ProtectHome = true;
            ProtectHostname = true;
            ProtectKernelLogs = true;
            ProtectKernelModules = true;
            ProtectKernelTunables = true;
            ProtectSystem = "strict";
            RemoveIPC = true;
            RestrictAddressFamilies = [
              "AF_INET"
              "AF_INET6"
              "AF_NETLINK"
              "AF_UNIX"
            ];

            RestrictNamespaces = true;
            RestrictSUIDSGID = true;
            SystemCallArchitectures = "native";
            UMask = "0077";
          };
        };
      };
    };
}
