{
  description = "GPUI desktop shell for codex-gui";

  nixConfig = {
    extra-substituters = [
      "https://oranc.li7g.com/ghcr.io/SuperKenVery/codex-gui-nix-cache"
    ];
    extra-trusted-public-keys = [
      "codex-gui-oranc-1:M5KXc8rneTuIystz2Z53emJGdiByGib+qNBva4PS2d0="
    ];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    bundlers = {
      url = "github:NixOS/bundlers";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      crane,
      rust-overlay,
      bundlers,
      ...
    }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      cargoPackage = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      version = cargoPackage.package.version;
      perSystem =
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };
          lib = pkgs.lib;
          rustToolchain = pkgs.rust-bin.stable.latest.default.override {
            extensions = [ "rust-src" ];
          };
          craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;
          codexGuiDefinition = import ./nix/codex-gui.nix {
            inherit craneLib pkgs version;
            icon = ./packaging/codex-gui.png;
            projectRoot = ./.;
            quickjsRuntimeSrc = ./crates/codex-code-mode-runtime-quickjs/src;
          };
          codex-gui = codexGuiDefinition.package;
          macosApp = import ./nix/macos_app.nix {
            codexGui = codex-gui;
            iconSource = ./packaging/codex-gui.png;
            inherit pkgs version;
          };
          macosArchive = import ./nix/macos_archive.nix {
            inherit macosApp pkgs version;
          };
          linuxPackages = lib.optionalAttrs pkgs.stdenv.isLinux {
            appimage = import ./nix/app_image.nix {
              inherit bundlers system;
              codexGui = codex-gui;
            };
            deb = import ./nix/deb.nix {
              inherit bundlers system;
              codexGui = codex-gui;
            };
            rpm = import ./nix/rpm.nix {
              inherit bundlers system;
              codexGui = codex-gui;
            };
          };
          darwinPackages = lib.optionalAttrs pkgs.stdenv.isDarwin {
            macos-app = macosApp;
            macos-archive = macosArchive;
          };
        in
        {
          packages = {
            default = codex-gui;
            inherit codex-gui;
          }
          // linuxPackages
          // darwinPackages;

          apps.default = {
            type = "app";
            program = "${codex-gui}/bin/codex-gui";
          };

          checks = {
            clippy = craneLib.cargoClippy (
              codexGuiDefinition.commonArgs
              // {
                inherit (codexGuiDefinition) cargoArtifacts;
                cargoClippyExtraArgs = "--all-targets -- --deny warnings";
              }
            );
            fmt = craneLib.cargoFmt {
              src = codexGuiDefinition.source;
            };
          };

          devShells.default = craneLib.devShell {
            checks = self.checks.${system};
            packages = if pkgs.stdenv.isLinux then (with pkgs; [ mangohud ]) else [ ];
            env = {
              RUST_BACKTRACE = "1";
              # Keep panic backtraces without making every recoverable anyhow
              # error capture a stack trace. GPUI creates recoverable errors in
              # hot rendering paths, where lib backtraces make scrolling jank.
              RUST_LIB_BACKTRACE = "0";
              RUST_LOG = "warn,codex_gui=debug";
            }
            // lib.optionalAttrs pkgs.stdenv.isLinux {
              LD_LIBRARY_PATH = lib.makeLibraryPath codexGuiDefinition.linuxRuntimeLibs;
            };
          };
        };
    in
    {
      packages = forAllSystems (system: (perSystem system).packages);
      apps = forAllSystems (system: (perSystem system).apps);
      checks = forAllSystems (system: (perSystem system).checks);
      devShells = forAllSystems (system: (perSystem system).devShells);
    };
}
