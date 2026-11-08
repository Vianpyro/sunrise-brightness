{
  description = "Adjusts screen brightness based on sunrise/sunset";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAll (pkgs: {
        default = pkgs.callPackage ./nix/package.nix { };
      });

      nixosModules.default = import ./nix/module.nix self;

      devShells = forAll (
        pkgs:
        let
          pkg = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        in
        {
          default = pkgs.mkShell {
            inputsFrom = [ pkg ];
            packages = with pkgs; [
              clippy
              rustfmt
              rust-analyzer
              ddcutil
            ];
            # `cargo run` has no patched RUNPATH, so point dlopen at the same libraries the package adds.
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath pkg.runtimeLibs + ":/run/opengl-driver/lib";
          };
        }
      );

      formatter = forAll (pkgs: pkgs.nixfmt);
    };
}
