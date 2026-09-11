{
  craneLib,
  icon,
  pkgs,
  projectRoot,
  quickjsRuntimeSrc,
  version,
}:
let
  lib = pkgs.lib;
  source = lib.cleanSourceWith {
    src = lib.cleanSource projectRoot;
    filter =
      path: type:
      !(type == "directory" && builtins.baseNameOf path == "nix")
      && (
        (craneLib.filterCargoSources path type)
        # Keep non-Rust files which are embedded at compile time or by
        # RustEmbed. Crane's default Cargo filter only retains Rust and Cargo
        # metadata.
        || lib.hasSuffix ".js" path
        || lib.hasSuffix ".json" path
        || lib.hasSuffix ".scm" path
        || lib.hasSuffix ".svg" path
        || lib.hasSuffix ".ps1" path
      );
  };
  dependencyDummySrc = craneLib.mkDummySrc {
    src = source;
    extraDummyScript = ''
      # codex-code-mode-host is a git dependency which imports the API
      # of this local [patch] replacement. Keep this one local crate's
      # implementation available while Crane stubs the application.
      rm -rf "$out/crates/codex-code-mode-runtime-quickjs/src"
      cp -R \
        ${quickjsRuntimeSrc} \
        "$out/crates/codex-code-mode-runtime-quickjs/src"
    '';
  };
  linuxRuntimeLibs = with pkgs; [
    libglvnd
    vulkan-loader
    wayland
  ];
  desktopItem = pkgs.makeDesktopItem {
    name = "codex-gui";
    desktopName = "Codex GUI";
    comment = "Native desktop client for Codex";
    exec = "codex-gui";
    icon = "codex-gui";
    categories = [ "Development" ];
    terminal = false;
  };
  commonArgs = {
    pname = "codex-gui";
    inherit version;
    src = source;
    strictDeps = true;
    # Distribution builds compile the release binaries; test builds belong in
    # a separate CI check and needlessly enlarge the shared Crane dependency
    # artifact.
    doCheck = false;
    nativeBuildInputs =
      (with pkgs; [
        cmake
        pkg-config
      ])
      ++ lib.optionals pkgs.stdenv.isDarwin [ pkgs.llvmPackages.lld ];
    buildInputs =
      lib.optionals pkgs.stdenv.isDarwin [
        pkgs.apple-sdk
        pkgs.libiconv
      ]
      ++ (with pkgs; [ openssl ])
      ++ lib.optionals pkgs.stdenv.isLinux (
        with pkgs;
        [
          fontconfig
          freetype
          libglvnd
          libx11
          libxcb
          libxkbcommon
          vulkan-loader
          wayland
        ]
      );
    NIX_LDFLAGS = lib.optionalString pkgs.stdenv.isLinux "-rpath ${lib.makeLibraryPath linuxRuntimeLibs}";
    postInstall = ''
      # Runtime executables must remain available from the package output.
      test -x "$out/bin/codex-gui"
      test -x "$out/bin/codex-code-mode-host"
      test -x "$out/bin/dsh-gui"

      install -Dm644 \
        ${desktopItem}/share/applications/codex-gui.desktop \
        "$out/share/applications/codex-gui.desktop"
      install -Dm644 \
        ${icon} \
        "$out/share/icons/hicolor/1024x1024/apps/codex-gui.png"
    '';
    meta = {
      description = "Native desktop GUI for Codex";
      homepage = "https://github.com/SuperKenVery/codex-gui";
      mainProgram = "codex-gui";
      platforms = lib.platforms.linux ++ lib.platforms.darwin;
    };
  };
  cargoArtifacts = craneLib.buildDepsOnly (
    commonArgs
    // {
      dummySrc = dependencyDummySrc;
      # This derivation only exports Cargo artifacts; the final executable
      # assertions belong to buildPackage below.
      postInstall = "";
    }
  );
  package = craneLib.buildPackage (
    commonArgs
    // {
      inherit cargoArtifacts;
    }
  );
in
{
  inherit
    cargoArtifacts
    commonArgs
    linuxRuntimeLibs
    package
    source
    ;
}
