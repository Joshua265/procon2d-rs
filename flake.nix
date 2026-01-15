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
        udevRulesPkg = pkgs.writeTextFile {
          name = "${pname}-udev-rules";
          destination = "/lib/udev/rules.d/60-${pname}.rules";
          text = ''
            ACTION=="add", SUBSYSTEM=="usb", ATTR{idVendor}=="057e", ATTR{idProduct}=="2069", \
              MODE="0660", GROUP="procon2d", \
              TAG+="systemd", ENV{SYSTEMD_WANTS}+="${pname}.service"
          '';
        };
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

          services.udev.packages = [udevRulesPkg];

          users.groups.procon2d = {};
          users.users.procon2d = {
            isSystemUser = true;
            group = "procon2d";
            extraGroups = ["input"]; # for /dev/uinput which is commonly root:input
          };

          systemd.services.${pname} = {
            description = "Pro Controller 2 driver (USB handshake + uinput translator)";
            wantedBy = ["multi-user.target"];
            after = ["systemd-udev-settle.service"];

            serviceConfig = {
              ExecStart = "${cfg.package}/bin/${pname} ${lib.escapeShellArgs cfg.extraArgs}";
              Restart = "on-failure";
              RestartSec = 1;

              CapabilityBoundingSet = ["CAP_SYS_RAWIO"];
              AmbientCapabilities = ["CAP_SYS_RAWIO"];
              NoNewPrivileges = true;

              DynamicUser = false;
              User = "procon2d";
              Group = "procon2d";
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
