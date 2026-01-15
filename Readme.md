# ProCon2-Daemon

Run the Nintendo **Pro Controller 2 (PID 0x2069)** on Linux without patching the kernel.

- 🔌 USB handshake → unlocks HID mode
- 🎮 Translates to a single **uinput** game-pad (ABS axes + buttons)
- 🧊 Ships a Nix flake (`nix run .`)
- 🛠️ Ships a **NixOS module** that installs udev rules + runs a **systemd service**

---

## Build & run (any distro)

```bash
git clone https://github.com/Joshua265/procon2d-rs.git
cargo r --release          # sudo or setcap cap_sys_rawio
```

Open `evtest` – pick **ProCon2 (virt)**. Sticks = ±32 767, buttons light up.

---

## Nix (one-shot run)

Build + run once (no installation):

```bash
nix run github:Joshua265/procon2d-rs
```

---

## NixOS (module + systemd service)

This flake exports a NixOS module that:

- installs the packaged udev rule (permissions + optional start-on-plug)
- defines a **system** unit `procon2d-rs.service`

### Flake-based NixOS config

Add the input and module, then enable the service:

```nix
# flake.nix (host)
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    procon2d-rs.url = "github:Joshua265/procon2d-rs";
  };

  outputs = { self, nixpkgs, procon2d, ... }: {
    nixosConfigurations.myhost = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        procon2d-rs.nixosModules.default

        ({ ... }: {
          services.procon2d-rs.enable = true;
          services.procon2d-rs.enableUdevRules = true;

          # optional
          # services.procon2d-rs.extraArgs = [ "--grab" ];
        })
      ];
    };
  };
}
```

Apply:

```bash
sudo nixos-rebuild switch --flake .#myhost
```

Check status/logs:

```bash
systemctl status procon2d-rs.service
journalctl -u procon2d-rs.service -f
```

---

## Stop Steam from reading the raw pad

Steam ≥2024 has no UI toggle for wired Nintendo pads, but you can blacklist it.

1. Exit Steam.

2. Edit `~/.steam/steam/config/config.vdf` and add:

   ```
   "controller_blacklist"      "057E/2069"
   ```

3. Save & restart Steam – only **ProCon2 (virt)** appears.

_(The daemon can also grab evdev for you; see **`--grab`** flag.)_

---

## Limitations

- Only tested on USB; Bluetooth not supported.
- L3/R3 buttons not mapped.
