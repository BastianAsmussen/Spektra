{ inputs, ... }:
let
  facterReport =
    lib: report: generate:
    let
      hasReport = builtins.pathExists report;
      name = baseNameOf report;
    in
    lib.mkMerge [
      (lib.mkIf hasReport { facter.reportPath = report; })
      (lib.mkIf (!hasReport) {
        warnings = [
          ''
            nix/${name} is missing, so no hardware has been detected.
            Generate it and commit it:

            ${generate}
          ''
        ];
      })
    ];
in
{
  flake.nixosModules.hardwareSpektra =
    { lib, ... }:
    let
      inherit (lib) mkMerge mkDefault;
    in
    {
      imports = [ inputs.nixos-facter-modules.nixosModules.facter ];

      config = mkMerge [
        (facterReport lib ./facter-spektra.json ''
          nix run github:nix-community/nixos-anywhere -- \
            --generate-hardware-config nixos-facter nix/facter-spektra.json \
            --flake .#spektra --target-host root@<ip>
        '')

        {
          boot.loader = {
            systemd-boot.enable = mkDefault false;
            efi.canTouchEfiVariables = mkDefault false;

            grub = {
              enable = mkDefault true;
              efiSupport = mkDefault false;
            };
          };

          services.qemuGuest.enable = mkDefault true;
          networking.useDHCP = mkDefault true;

          nixpkgs.hostPlatform = mkDefault "x86_64-linux";
        }
      ];
    };

  flake.nixosModules.hardwareRadioNode =
    { lib, ... }:
    let
      inherit (lib) mkMerge mkDefault;
    in
    {
      imports = [
        inputs.nixos-facter-modules.nixosModules.facter
        inputs.nixos-hardware.nixosModules.raspberry-pi-5
      ];

      config = mkMerge [
        (facterReport lib ./facter-radio-node.json ''
          ssh <node> 'sudo nixos-facter' > nix/facter-radio-node.json
        '')

        {
          boot.loader = {
            grub.enable = mkDefault false;
            generic-extlinux-compatible.enable = mkDefault true;
          };

          networking.useDHCP = mkDefault true;

          nixpkgs.hostPlatform = mkDefault "aarch64-linux";
        }
      ];
    };
}
