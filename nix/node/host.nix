{ inputs, self, ... }:
{
  flake.nixosConfigurations.radio-node = inputs.nixpkgs.lib.nixosSystem {
    specialArgs = { inherit inputs self; };

    modules = [ self.nixosModules.hostRadioNode ];
  };

  flake.nixosModules.hostRadioNode =
    { config, lib, ... }:
    let
      cfg = config.radio-node;

      inherit (lib) mkOption types;
    in
    {
      imports = [
        inputs.disko.nixosModules.disko
        self.diskoConfigurations.radio-node

        self.nixosModules.hardwareRadioNode
        self.nixosModules.nodeAgent
      ];

      options.radio-node = {
        server = mkOption {
          type = types.str;
          default = "http://spektra:50051";
          example = "https://spektra.asmussen.tech:50051";
          description = "gRPC endpoint the agent reports to.";
        };

        sshKeys = mkOption {
          type = types.listOf types.str;
          default = [ ];
          description = "Authorized keys for the maintenance user.";
        };
      };

      config = {
        networking = {
          hostName = "radio-node";
          firewall.allowedTCPPorts = [ 22 ];
        };

        zramSwap.enable = lib.mkDefault true;
        services = {
          spektra-node-agent = {
            inherit (cfg) server;

            enable = true;
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

        nix.settings.experimental-features = [
          "nix-command"
          "flakes"
        ];

        system.stateVersion = "25.11";
      };
    };
}
