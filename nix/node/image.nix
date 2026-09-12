{ inputs, self, ... }:
let
  inherit (inputs.nixpkgs) lib;

  mkImage =
    name: system:
    (system.extendModules {
      modules = [
        "${inputs.nixpkgs}/nixos/modules/installer/sd-card/sd-image-aarch64.nix"
        {
          image.baseName = name;

          sdImage = {
            compressImage = false;
            firmwareSize = 256;
          };

          boot.supportedFilesystems.zfs = lib.mkForce false;
        }
      ];
    }).config.system.build.sdImage;
in
{
  flake.images = lib.mapAttrs mkImage (
    lib.filterAttrs (name: _: lib.hasPrefix "radio-node" name) self.nixosConfigurations
  );
}
