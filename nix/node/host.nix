{
  inputs,
  self,
  withSystem,
  ...
}:
let
  inherit (inputs.nixpkgs) lib;

  adminKeys = [
    "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAACAQDUNgmpEFxwqSo0Jm2Y2q0gZMAzkp/f94PMW4s60JCC472dfLAgVOCtEAibak8hkt19BX7YPQHutQfgBKgK5GOzq6Ma/KXd/4UqPvRV/wtBQ1nrV3WzXrqSjhPYr2CA0xZ3A6xmHdCQvK7g9ifx6jFVSktkpSnbnvkczy2+ISsbQXlxrpNcK6Lq7b56tydC9nkQdori4b3TbNNR6T6Li5wrnIj0Mgz8BzfuRaZdofwjpQ9gq31PO/aIeIeuFc1SYwaQwBAY/EZDWmGrAq/HWusTrBLaSRzDKZBYDbOIbQX8qTwV4YGOZts22ZobZBjsx6VGhpmo+K0M3QQtW66MpvtxJ8QlrnET7NcOhGOrn3iddMmsQZ1oPnsR5AgUqFyjnWq2tcdFRX5HVV5XM5qVG2vzBGz4bAoXJmNQyPvnjqmleYgxzrNxHEhaDSMlY4pPV4qOkWPCk96+uQFoGI6dUiqDmw0qKrr8Dhqki0owgGb4FV7tGkVc00xlsNnVzXXYtUzOF9tJp0lKstAWXG0nvOmNRyafG2uucyuR5p0Q9jCX3hIeCdOJWkAGHacHeykAuc1c8hM4o6sCbx+qcSsojNbIFFhqzQUNmUNsDRDbNJq9XAlOsB6hBjuS2B42v4Tn0+jPY+eka1ZJ/1mD4/O5Tqg0B+AzAREdz3MSqNwzX6ASLQ== bastian"
  ];

  nodes = {
    radio-node-1.services.spektra-node-agent.gainDb = 30.0;
    radio-node-2.services.spektra-node-agent.gainDb = 30.0;
    radio-node-3.services.spektra-node-agent.gainDb = 30.0;

    radio-node-airspy.services.spektra-node-agent = {
      receiver = "airspy-mini";
      sampleRateHz = 6000000;
      gainDb = 21.0;
    };
  };

  mkNode =
    hostName: settings:
    lib.nixosSystem {
      specialArgs = { inherit inputs self; };

      modules = [
        self.nixosModules.hostRadioNode
        settings
        {
          radio-node = {
            inherit hostName;

            sshKeys = adminKeys;
          };

          services.spektra-node-agent.nodeName = hostName;
        }
      ];
    };
in
{
  flake = {
    nixosConfigurations = lib.mapAttrs mkNode nodes;
    nixosModules.hostRadioNode =
      {
        config,
        lib,
        ...
      }:
      let
        cfg = config.radio-node;

        inherit (lib) mkOption types mkDefault;
      in
      {
        imports = [
          self.nixosModules.hardwareRadioNode
          self.nixosModules.nodeAgent
        ];

        options.radio-node = {
          hostName = mkOption {
            type = types.str;
            default = "radio-node";
            description = "Host name, and the name the agent registers under.";
          };

          server = mkOption {
            type = types.str;
            default = "https://spektra.asmussen.tech";
            example = "http://spektra:50051";
            description = "gRPC endpoint the agent reports to.";
          };

          sshKeys = mkOption {
            type = types.listOf types.str;
            default = [ ];
            description = "Authorized keys for the maintenance user.";
          };

          wifi = {
            ssid = mkOption {
              type = types.nullOr types.str;
              default = null;
              description = "Wireless network to fall back to when no cable is present.";
            };

            secretsFile = mkOption {
              type = types.nullOr types.str;
              default = "/var/lib/wpa_supplicant/secrets";
              description = "File holding `psk=<64 hex digits>`, read outside the store.";
            };
          };
        };

        config = {
          boot = {
            kernelPackages = withSystem "x86_64-linux" (
              { pkgs, ... }:
              let
                cross = pkgs.pkgsCross.aarch64-multiplatform;
              in
              cross.linuxPackagesFor (
                cross.callPackage "${inputs.nixos-hardware}/raspberry-pi/common/kernel.nix" { rpiVersion = 5; }
              )
            );

            initrd.systemd.tpm2.enable = false;
            kernelParams = [ "cfg80211.ieee80211_regdom=DK" ];
          };

          hardware.raspberry-pi.firmware.uboot.enable = true;

          fileSystems = {
            "/" = {
              device = mkDefault "/dev/disk/by-label/NIXOS_SD";
              fsType = mkDefault "ext4";
            };

            "/boot/firmware" = {
              device = mkDefault "/dev/disk/by-label/FIRMWARE";
              fsType = mkDefault "vfat";
              options = mkDefault [
                "nofail"
                "noauto"
              ];
            };
          };

          networking = {
            inherit (cfg) hostName;

            firewall.allowedTCPPorts = [ 22 ];
            useNetworkd = true;
            useDHCP = false;
            wireless = {
              enable = true;
              allowAuxiliaryImperativeNetworks = true;

              secretsFile = lib.mkIf (cfg.wifi.ssid != null) cfg.wifi.secretsFile;
              networks = lib.mkIf (cfg.wifi.ssid != null) {
                ${cfg.wifi.ssid}.pskRaw = "ext:psk";
              };
            };
          };

          systemd.network = {
            wait-online.anyInterface = true;
            networks = {
              "10-wired" = {
                matchConfig.Type = "ether";
                networkConfig.DHCP = "yes";
                dhcpV4Config.RouteMetric = 10;
                ipv6AcceptRAConfig.RouteMetric = 10;
              };

              "20-wireless" = {
                matchConfig.Type = "wlan";
                networkConfig.DHCP = "yes";
                dhcpV4Config.RouteMetric = 600;
                ipv6AcceptRAConfig.RouteMetric = 600;
              };
            };
          };

          zramSwap.enable = mkDefault true;
          services = {
            spektra-node-agent = {
              inherit (cfg) server;

              enable = true;
              package = withSystem "x86_64-linux" ({ config, ... }: config.packages.node-agent-aarch64);
            };

            openssh = {
              enable = true;
              settings = {
                PasswordAuthentication = false;
                PermitRootLogin = "no";
              };
            };

            journald.extraConfig = "Storage=volatile";
          };

          users.users.maintainer = {
            isNormalUser = true;
            extraGroups = [
              "wheel"
              "plugdev"
            ];

            openssh.authorizedKeys.keys = cfg.sshKeys;
          };

          security.sudo.extraRules = [
            {
              users = [ "maintainer" ];
              commands = [
                {
                  command = "ALL";
                  options = [ "NOPASSWD" ];
                }
              ];
            }
          ];

          nix.settings = {
            experimental-features = [
              "nix-command"
              "flakes"
            ];

            trusted-users = [ "maintainer" ];
          };

          system.stateVersion = "25.11";
        };
      };
  };
}
