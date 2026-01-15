{
  description = "Userspace Pro Controller 2 (Switch 2) daemon — USB handshake + uinput translator";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    ...
  }: let
    pname = "procon2d-rs";
    version = "0.1.0";

    mkPkgs = system:
      import nixpkgs {
        inherit system;
        config.allowUnfree = true;
      };

    mkDrv = system: let
      pkgs = mkPkgs system;
    in
      pkgs.rustPlatform.buildRustPackage {
        inherit pname version;
        src = ./.;

        cargoHash = "sha256-+Awdya4v9w9/NNy46WM+u9xnNnOwUdeLbDMkGB064Ok=";

        nativeBuildInputs = [pkgs.pkg-config];
        buildInputs = [pkgs.libusb1 pkgs.hidapi];

        postInstall = ''
          # Ship a udev rule file inside the package so NixOS can pick it up via services.udev.packages.
          # Use a priority < 99 so TAG+="uaccess" applies early enough.
          mkdir -p $out/lib/udev/rules.d
          cat > $out/lib/udev/rules.d/60-${pname}.rules <<'EOF'
          # Nintendo Pro Controller 2 (example IDs from your original rule)
          ACTION=="add", SUBSYSTEM=="usb", ATTR{idVendor}=="057e", ATTR{idProduct}=="2069", \
            TAG+="uaccess", GROUP="input", \
            TAG+="systemd", ENV{SYSTEMD_WANTS}+="${pname}.service"
          EOF
        '';

        meta = with pkgs.lib; {
          description = "Daemon that enables Nintendo Pro Controller 2 on Linux via libusb + uinput";
          homepage = "https://github.com/Joshua265/procon2-daemon";
          license = licenses.mit;
          maintainers = [maintainers.Joshua265];
          platforms = platforms.linux;
        };
      };
  in
    # Per-system outputs (packages, devShells, …)
    (flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = mkPkgs system;
        drv = mkDrv system;
      in {
        packages.default = drv;
        packages.${pname} = drv;

        devShells.default = pkgs.mkShell {
          packages = [
            pkgs.rustc
            pkgs.cargo
            pkgs.cargo-edit
            pkgs.pkg-config
            pkgs.libusb1
            pkgs.hidapi
          ];
          RUST_BACKTRACE = "1";
        };
      }
    ))
    // {
      # NixOS module output: importing this lets consumers enable a running systemd service.
      nixosModules.default = {
        config,
        lib,
        pkgs,
        ...
      }: let
        # NOTE: hyphenated option names require quotes when setting them in config
        cfg = config.services.${pname};

        system = pkgs.stdenv.hostPlatform.system;
        defaultPkg =
          self.packages.${system}.${pname} or self.packages.${system}.default;
      in {
        options.services.${pname} = {
          enable = lib.mkEnableOption "Pro Controller 2 daemon";

          package = lib.mkOption {
            type = lib.types.package;
            default = defaultPkg;
            defaultText = lib.literalExpression "inputs.procon2d.packages.${pkgs.stdenv.hostPlatform.system}.default";
            description = "Package that provides ${pname}.";
          };

          extraArgs = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [];
            description = "Extra CLI args for the daemon.";
          };

          enableUdevRules = lib.mkOption {
            type = lib.types.bool;
            default = true;
            description = "Install the packaged udev rules (permissions + optional systemd start-on-plug).";
          };
        };

        config = lib.mkIf cfg.enable {
          # Ensure uinput exists (module on many kernels)
          boot.kernelModules = ["uinput"];

          # Pull in udev rules shipped at $pkg/lib/udev/rules.d
          services.udev.packages = lib.mkIf cfg.enableUdevRules [cfg.package];

          systemd.services.${pname} = {
            description = "Pro Controller 2 driver (USB handshake + uinput translator)";
            wantedBy = ["multi-user.target"];
            after = ["systemd-udev-settle.service"];
            # If your daemon depends on networking, add:
            # after = [ "network-online.target" ];
            # wants = [ "network-online.target" ];

            serviceConfig = {
              ExecStart = "${cfg.package}/bin/${pname} ${lib.escapeShellArgs cfg.extraArgs}";
              Restart = "on-failure";
              RestartSec = 1;

              # Likely needed for raw USB access; adjust based on what the daemon actually does.
              CapabilityBoundingSet = ["CAP_SYS_RAWIO"];
              AmbientCapabilities = ["CAP_SYS_RAWIO"];
              NoNewPrivileges = true;

              # Run with a dynamic user, but allow access to /dev/uinput (typically group "input")
              DynamicUser = true;
              SupplementaryGroups = ["input"];

              # Nice defaults
              StateDirectory = pname;
              PrivateTmp = true;
              ProtectSystem = "strict";
              ProtectHome = true;
            };
          };
        };
      };
    };
}
