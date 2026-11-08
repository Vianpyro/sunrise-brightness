# Sunrise Brightness

Automatically adjusts your screen brightness throughout the day based on the sun's position. Runs in the system tray and adapts to weather conditions.

![Curve Editor](illustration.png)

## Features

- **Solar-powered brightness**: uses the NREL Solar Position Algorithm (±0.0003°) to compute precise sunrise, sunset, and solar elevation for your location — no API key needed
- **Weather adaptation**: dims your screens when it's cloudy or rainy using Open-Meteo's free forecast (no API key)
- **Interactive curve editor**: drag control points to customize your brightness profile, with 7 presets (Natural, Linear, Night Owl, Relax, Cinema, Paper, Early Bird)
- **Per-monitor support**: set each display to follow the global curve, apply an offset, use a fully custom curve, or be ignored entirely
- **System tray**: runs silently in the background, opens a settings panel on click
- **Auto-start**: optional launch on boot
- **Auto-update check**: notifies you when a new version is available on GitHub

## Installation

### Linux (.deb)

Download the `.deb` from the [latest release](https://github.com/Vianpyro/sunrise-brightness/releases/latest) and install:

```bash
sudo dpkg -i sunrise-brightness_*.deb
```

This automatically installs the binary to `/usr/bin/` and pulls in runtime dependencies.

### Linux (manual binary)

Download the binary from the [latest release](https://github.com/Vianpyro/sunrise-brightness/releases/latest), then:

```bash
# Install runtime dependencies
sudo apt install libgtk-3-0 libxdo3 libappindicator3-1 libssl3

# Install the binary
sudo install -m 755 sunrise-brightness-linux /usr/local/bin/sunrise-brightness

# Run it
sunrise-brightness
```

### NixOS

The repository is a flake. Add it as an input and enable the module:

```nix
# flake.nix
inputs.sunrise-brightness = {
  url = "github:Vianpyro/sunrise-brightness";
  inputs.nixpkgs.follows = "nixpkgs";
};

# in your NixOS configuration (modules = [ inputs.sunrise-brightness.nixosModules.default … ])
programs.sunrise-brightness.enable = true;
```

The module installs the app and sets `hardware.i2c.enable = true`. That is what makes external monitors work, as explained below. To try it without installing anything, run `nix run github:Vianpyro/sunrise-brightness`. Turn on "Start on startup" only once the app is installed: the autostart entry points at your profile's stable path rather than the `/nix/store` path, so it survives upgrades and garbage collection.

### External monitors (DDC/CI) on Linux

Laptop panels are driven through `/sys/class/backlight`. External monitors are driven over DDC/CI, so the app changes the monitor's real backlight instead of darkening the image. This needs:

- the `i2c-dev` kernel module (`sudo modprobe i2c-dev`, made permanent through `/etc/modules-load.d/`);
- read/write access to `/dev/i2c-*`. Usually that means joining the `i2c` group (`sudo usermod -aG i2c $USER`, then log in again). On NixOS, `hardware.i2c.enable` grants it to the logged-in user automatically;
- DDC/CI enabled in the monitor's on-screen menu (it usually is by default).

`ddcutil detect` shows which monitors answer. Only adapters that can carry a display are probed: motherboard SMBus is never touched.

**KDE Plasma:** Plasma 6 also drives DDC/CI monitors through its own brightness slider. Both work, but each overwrites the other's value. If you want Sunrise Brightness to be the only one in charge, turn off DDC/CI for those screens in Plasma (`allowDdcCi` in `~/.config/kwinoutputconfig.json`).

### Windows

Download `sunrise-brightness-windows.exe` from the [latest release](https://github.com/Vianpyro/sunrise-brightness/releases/latest) and run it. No installation needed, the binary is self-contained.

## Configuration

Settings are stored in a TOML file at the platform-appropriate config directory:

| Platform | Path                                              |
| -------- | ------------------------------------------------- |
| Linux    | `~/.config/sunrise-brightness/config.toml`        |
| Windows  | `%APPDATA%\sunrise-brightness\config\config.toml` |

All settings can be changed through the GUI. Example config:

```toml
latitude = 45
longitude = 0
update_interval_secs = 180
start_on_startup = true
weather_adaptive = true
cloud_attenuation = 0.5

[global_curve]
preset = "natural"
[[global_curve.points]]
position = 0.0
brightness = 0.0
# ... more points
```

If `latitude`/`longitude` are omitted, your location is auto-detected from your IP address.

## Building from source

With Nix, `nix develop` gives a shell with the toolchain and every library, and `nix build` builds the package. Otherwise:

```bash
# Linux: install build dependencies
sudo apt install build-essential pkg-config libgtk-3-dev libappindicator3-dev libxdo-dev libssl-dev libwayland-dev

# Build
cargo build --release

# The binary is at target/release/sunrise-brightness
```

## Build verification

All release binaries include [SLSA build provenance attestations](https://slsa.dev/). You can verify any binary with:

```bash
gh attestation verify sunrise-brightness-linux --repo Vianpyro/sunrise-brightness
```
