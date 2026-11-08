# Builds sunrise-brightness from this source tree. Used by ../flake.nix; can
# also be called directly with `pkgs.callPackage ./nix/package.nix { }`.
{
  lib,
  rustPlatform,
  pkg-config,
  copyDesktopItems,
  makeDesktopItem,
  addDriverRunpath,
  # linked (DT_NEEDED)
  gtk3,
  glib,
  gdk-pixbuf,
  xdotool,
  # dlopen'd at runtime, see postFixup
  libGL,
  wayland,
  libxkbcommon,
  libayatana-appindicator,
  libx11,
  libxcb,
  libxi,
  libxrender,
  libxcursor,
}:
let
  cargoToml = lib.importTOML ../Cargo.toml;

  # eframe's glow backend and the tray-icon crate resolve these with dlopen, so
  # the linker never records them and the binary would fail to start on NixOS.
  runtimeLibs = [
    libGL
    wayland
    libxkbcommon
    libayatana-appindicator
    libx11
    libxcb
    libxi
    libxrender
    libxcursor
  ];
in
rustPlatform.buildRustPackage {
  pname = cargoToml.package.name;
  inherit (cargoToml.package) version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../src
      ../sunrise-brightness.svg
    ];
  };

  # Reads the hashes straight from Cargo.lock, so a dependency bump needs no change here.
  cargoLock.lockFile = ../Cargo.lock;

  nativeBuildInputs = [
    pkg-config
    copyDesktopItems
  ];

  buildInputs = [
    gtk3
    glib
    gdk-pixbuf
    xdotool
  ];

  # driverLink is where NixOS exposes the real libGL/libEGL implementation
  # (Mesa, or NVIDIA's); libglvnd only dispatches to it.
  postFixup = ''
    patchelf --add-rpath ${
      lib.makeLibraryPath runtimeLibs + ":${addDriverRunpath.driverLink}/lib"
    } $out/bin/sunrise-brightness
  '';

  postInstall = ''
    install -Dm644 sunrise-brightness.svg \
      $out/share/icons/hicolor/scalable/apps/sunrise-brightness.svg
  '';

  # Same as ../sunrise-brightness.desktop.
  desktopItems = [
    (makeDesktopItem {
      name = "sunrise-brightness";
      exec = "sunrise-brightness";
      icon = "sunrise-brightness";
      desktopName = "Sunrise Brightness";
      comment = "Adjusts screen brightness based on sunrise/sunset";
      categories = [
        "Utility"
        "System"
      ];
      keywords = [
        "brightness"
        "display"
        "screen"
        "sunset"
        "sunrise"
      ];
      startupNotify = false;
    })
  ];

  passthru = { inherit runtimeLibs; };

  meta = {
    inherit (cargoToml.package) description;
    homepage = "https://github.com/Vianpyro/sunrise-brightness";
    license = lib.licenses.mit;
    mainProgram = "sunrise-brightness";
    platforms = lib.platforms.linux;
  };
}
