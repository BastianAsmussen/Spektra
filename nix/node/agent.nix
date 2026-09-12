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

      args = [
        "--name"
        cfg.nodeName
        "--receiver"
        cfg.receiver
        "--sample-rate-hz"
        (toString cfg.sampleRateHz)
        "--latitude"
        (toString cfg.latitude)
        "--longitude"
        (toString cfg.longitude)
        "--antenna"
        cfg.antenna
        "--dwell-seconds"
        (toString cfg.dwellSeconds)
      ]
      ++ lib.optionals (cfg.gainDb != null) [
        "--gain-db"
        (toString cfg.gainDb)
      ]
      ++ cfg.extraArgs;
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

        nodeName = mkOption {
          type = types.str;
          default = "spektra-node";
          description = "Name the node registers itself under.";
        };

        receiver = mkOption {
          type = types.enum [
            "rtl-sdr"
            "airspy-mini"
            "synthetic"
          ];

          default = "rtl-sdr";
          description = "Receiver family the agent opens through SoapySDR.";
        };

        sampleRateHz = mkOption {
          type = types.ints.positive;
          default = 2400000;
          description = ''
            Sample rate requested from the receiver. The RTL-SDR tops out at
            2.4 MSPS; the Airspy Mini offers 6 and 3 MSPS and coerces anything
            else, so set it explicitly there.
          '';
        };

        gainDb = mkOption {
          type = types.nullOr types.number;
          default = null;
          description = ''
            Fixed gain in dB. Null hands gain to the driver's AGC, which makes
            the level metric a measurement of the AGC rather than of the signal.
          '';
        };

        latitude = mkOption {
          type = types.number;
          default = 57.048;
          description = "Reported latitude of the node.";
        };

        longitude = mkOption {
          type = types.number;
          default = 9.921;
          description = "Reported longitude of the node.";
        };

        antenna = mkOption {
          type = types.str;
          default = "fixed dipole";
          description = "Antenna description reported at registration.";
        };

        dwellSeconds = mkOption {
          type = types.number;
          default = 1.0;
          description = "Seconds spent on each assigned channel per rotation.";
        };

        extraArgs = mkOption {
          type = types.listOf types.str;
          default = [ ];
          example = [
            "--fft-size"
            "65536"
          ];

          description = "Additional command line arguments passed to the agent.";
        };

        environmentFile = mkOption {
          type = types.nullOr types.str;
          default = "/var/lib/spektra-node-agent/enrollment.env";
          description = ''
            Path read for `SPEKTRA_ENROLLMENT_TOKEN`, which is only consulted on
            the node's first registration. Optional to systemd, so a node that
            already holds a credential starts without it.
          '';
        };
      };

      config = lib.mkIf cfg.enable {
        hardware.rtl-sdr.enable = true;

        services.udev.packages = [ pkgs.airspy ];

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

            SOAPY_SDR_PLUGIN_PATH = "${pkgs.soapysdr-with-plugins}/${pkgs.soapysdr-with-plugins.searchPath}";
          };

          startLimitBurst = 5;
          startLimitIntervalSec = 300;
          serviceConfig = {
            ExecStart = "${lib.getExe cfg.package} ${lib.escapeShellArgs args}";
            EnvironmentFile = lib.mkIf (cfg.environmentFile != null) [ "-${cfg.environmentFile}" ];
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
