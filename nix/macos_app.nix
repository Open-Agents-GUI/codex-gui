{
  codexGui,
  iconSource,
  pkgs,
  version,
}:
let
  infoPlist = pkgs.writeText "Info.plist" ''
    <?xml version="1.0" encoding="UTF-8"?>
    <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
    <plist version="1.0">
    <dict>
      <key>CFBundleDisplayName</key>
      <string>Codex GUI</string>
      <key>CFBundleExecutable</key>
      <string>codex-gui</string>
      <key>CFBundleIdentifier</key>
      <string>io.github.superkenvery.codex-gui</string>
      <key>CFBundleIconFile</key>
      <string>codex-gui</string>
      <key>CFBundleInfoDictionaryVersion</key>
      <string>6.0</string>
      <key>CFBundleName</key>
      <string>Codex GUI</string>
      <key>CFBundlePackageType</key>
      <string>APPL</string>
      <key>CFBundleShortVersionString</key>
      <string>${version}</string>
      <key>CFBundleVersion</key>
      <string>${version}</string>
      <key>LSMinimumSystemVersion</key>
      <string>12.0</string>
      <key>NSHighResolutionCapable</key>
      <true/>
      <key>NSPrincipalClass</key>
      <string>NSApplication</string>
    </dict>
    </plist>
  '';
in
pkgs.runCommand "codex-gui-${version}-macos-app"
  {
    nativeBuildInputs = with pkgs; [
      imagemagick
      libicns
      macdylibbundler
    ];
  }
  ''
    # dylibbundler 1.0.4 passes paths to otool through a shell without
    # preserving spaces, so bundle under a temporary space-free name and
    # rename only after signing.
    app="$out/CodexGUI.app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Frameworks" "$app/Contents/Resources"
    cp ${codexGui}/bin/codex-gui "$app/Contents/MacOS/codex-gui"
    cp ${codexGui}/bin/codex-code-mode-host "$app/Contents/MacOS/codex-code-mode-host"

    icon_dir="$TMPDIR/codex-gui-icons"
    mkdir -p "$icon_dir"
    for size in 16 32 128 256 512 1024; do
      magick ${iconSource} -resize "''${size}x''${size}" "$icon_dir/icon-''${size}.png"
    done
    png2icns \
      "$app/Contents/Resources/codex-gui.icns" \
      "$icon_dir/icon-16.png" \
      "$icon_dir/icon-32.png" \
      "$icon_dir/icon-128.png" \
      "$icon_dir/icon-256.png" \
      "$icon_dir/icon-512.png" \
      "$icon_dir/icon-1024.png"

    cp ${infoPlist} "$app/Contents/Info.plist"
    chmod 755 "$app/Contents/MacOS/codex-gui" "$app/Contents/MacOS/codex-code-mode-host"

    # Sign the completed app ourselves. dylibbundler's ad-hoc signer is not
    # on PATH in a sandboxed Nix build.
    dylibbundler -ns -od -b \
      -x "$app/Contents/MacOS/codex-gui" \
      -d "$app/Contents/Frameworks" \
      -p @executable_path/../Frameworks/
    dylibbundler -ns -of -cd -b \
      -x "$app/Contents/MacOS/codex-code-mode-host" \
      -d "$app/Contents/Frameworks" \
      -p @executable_path/../Frameworks/

    /usr/bin/codesign --force --deep --sign - "$app"
    mv "$app" "$out/Codex GUI.app"
  ''
