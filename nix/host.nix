{ inputs, self, ... }:
{
  flake.nixosConfigurations.spektra = inputs.nixpkgs.lib.nixosSystem {
    specialArgs = { inherit inputs self; };

    modules = [
      self.nixosModules.hostSpektra

      {
        spektra-host = {
          domain = "spektra.asmussen.tech";
          acmeEmail = "spektra@asmussen.tech";

          adminKeys = [
            "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAACAQDUNgmpEFxwqSo0Jm2Y2q0gZMAzkp/f94PMW4s60JCC472dfLAgVOCtEAibak8hkt19BX7YPQHutQfgBKgK5GOzq6Ma/KXd/4UqPvRV/wtBQ1nrV3WzXrqSjhPYr2CA0xZ3A6xmHdCQvK7g9ifx6jFVSktkpSnbnvkczy2+ISsbQXlxrpNcK6Lq7b56tydC9nkQdori4b3TbNNR6T6Li5wrnIj0Mgz8BzfuRaZdofwjpQ9gq31PO/aIeIeuFc1SYwaQwBAY/EZDWmGrAq/HWusTrBLaSRzDKZBYDbOIbQX8qTwV4YGOZts22ZobZBjsx6VGhpmo+K0M3QQtW66MpvtxJ8QlrnET7NcOhGOrn3iddMmsQZ1oPnsR5AgUqFyjnWq2tcdFRX5HVV5XM5qVG2vzBGz4bAoXJmNQyPvnjqmleYgxzrNxHEhaDSMlY4pPV4qOkWPCk96+uQFoGI6dUiqDmw0qKrr8Dhqki0owgGb4FV7tGkVc00xlsNnVzXXYtUzOF9tJp0lKstAWXG0nvOmNRyafG2uucyuR5p0Q9jCX3hIeCdOJWkAGHacHeykAuc1c8hM4o6sCbx+qcSsojNbIFFhqzQUNmUNsDRDbNJq9XAlOsB6hBjuS2B42v4Tn0+jPY+eka1ZJ/1mD4/O5Tqg0B+AzAREdz3MSqNwzX6ASLQ== bastian"
          ];

          deployKeys = [
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICscG2Nfdfvex5oJDUoTYyobDXn0TsCJJ5BtSR9RlNG2 spektra-deploy"
          ];
        };
      }
    ];
  };

  flake.nixosModules.hostSpektra =
    {
      config,
      lib,
      ...
    }:
    let
      cfg = config.spektra-host;

      inherit (lib) mkOption types mkIf;
    in
    {
      imports = [
        inputs.disko.nixosModules.disko
        self.diskoConfigurations.spektra

        inputs.sops-nix.nixosModules.sops

        self.nixosModules.hardwareSpektra
        self.nixosModules.spektra
      ];

      options.spektra-host = {
        domain = mkOption {
          type = types.nullOr types.str;
          default = null;
          example = "spektra.asmussen.tech";
          description = ''
            Domain the web client and the gRPC ingest are served on. When set,
            Caddy terminates TLS for both and the two application ports stay on
            loopback. When null they are opened directly, in cleartext.
          '';
        };

        acmeEmail = mkOption {
          type = types.nullOr types.str;
          default = null;
          description = "Contact address Caddy registers with Let's Encrypt. Required when `domain` is set.";
        };

        adminKeys = mkOption {
          type = types.listOf types.str;
          default = [ ];
          description = "Authorized keys for the interactive admin user.";
        };

        deployKeys = mkOption {
          type = types.listOf types.str;
          default = [ ];
          description = ''
            Authorized keys for the deploy user the CI workflow logs in as.
            One key per workflow, never a copy of an admin key.
          '';
        };
      };

      config = {
        assertions = [
          {
            assertion = cfg.domain == null || cfg.acmeEmail != null;
            message = "spektra-host: setting domain also requires acmeEmail.";
          }
        ];

        networking = {
          hostName = "spektra";

          firewall.allowedTCPPorts =
            if cfg.domain == null then
              [
                config.services.spektra.port
                config.services.spektra.grpcPort
              ]
            else
              [
                80
                443
              ];
        };

        services = {
          spektra = {
            enable = true;
            host = if cfg.domain == null then "0.0.0.0" else "127.0.0.1";
            grpcHost = if cfg.domain == null then "0.0.0.0" else "127.0.0.1";
            environmentFile = config.sops.secrets.spektra_env.path;
            ntfyUrl = "https://ntfy.sh";
          };

          caddy = mkIf (cfg.domain != null) {
            enable = true;
            email = cfg.acmeEmail;

            virtualHosts.${cfg.domain}.extraConfig = ''
              @ingest path /spektra.v1.NodeIngest/*
              reverse_proxy @ingest h2c://127.0.0.1:${toString config.services.spektra.grpcPort}

              reverse_proxy 127.0.0.1:${toString config.services.spektra.port}
            '';
          };

          openssh = {
            enable = true;
            settings = {
              PasswordAuthentication = false;
              PermitRootLogin = "no";
            };
          };
        };

        users.users = {
          bastian = {
            isNormalUser = true;
            extraGroups = [ "wheel" ];
            openssh.authorizedKeys.keys = cfg.adminKeys;
          };

          deploy = {
            isNormalUser = true;
            extraGroups = [ "wheel" ];
            openssh.authorizedKeys.keys = cfg.deployKeys;
          };
        };

        security.sudo.extraRules = [
          {
            users = [
              "bastian"
              "deploy"
            ];
            commands = [
              {
                command = "ALL";
                options = [ "NOPASSWD" ];
              }
            ];
          }
        ];

        sops = {
          age.sshKeyPaths = [ "/etc/ssh/ssh_host_ed25519_key" ];

          secrets = {
            spektra_env = {
              format = "binary";
              sopsFile = ./secrets/spektra.env;
              mode = "0400";
              restartUnits = [ "spektra.service" ];
            };
          };
        };

        nix.settings = {
          experimental-features = [
            "nix-command"
            "flakes"
          ];

          trusted-users = [
            "root"
            "bastian"
            "deploy"
          ];
        };

        zramSwap.enable = lib.mkDefault true;

        system.stateVersion = "25.11";
      };
    };
}
