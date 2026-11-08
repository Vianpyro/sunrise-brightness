# NixOS module: `programs.sunrise-brightness.enable = true;`
#
# Installs the app and gives the logged-in user access to /dev/i2c-*, which is
# what lets it set the real backlight of external monitors over DDC/CI.
# Laptop panels go through /sys/class/backlight and logind, which work without
# anything extra.
self:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.sunrise-brightness;
in
{
  options.programs.sunrise-brightness = {
    enable = lib.mkEnableOption "Sunrise Brightness, sun-following screen brightness";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "sunrise-brightness.packages.\${system}.default";
      description = "The sunrise-brightness package to install.";
    };

    ddcci = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Enable `hardware.i2c` so external monitors can be driven over DDC/CI.
        It loads `i2c-dev` and tags the device nodes `uaccess`, so the seated
        user gets them without joining a group.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];
    hardware.i2c.enable = lib.mkIf cfg.ddcci true;
  };
}
