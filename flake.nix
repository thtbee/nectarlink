{
  description = "NectarLink - Android and Linux PC working as one";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    let
      packageBuilder = system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          qtEnv = pkgs.qt6.env "qt6-env" [
            pkgs.qt6.qtbase
            pkgs.qt6.qtdeclarative
            pkgs.qt6.qtsvg
            pkgs.qt6.qtwayland
            pkgs.qt6.qtshadertools
            pkgs.qt6.qttools
          ];
        in pkgs.rustPlatform.buildRustPackage {
          pname = "nectarlink";
          version = "0.1.5";
          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          nativeBuildInputs = with pkgs; [
            pkg-config
            cmake
            ninja
            qt6.wrapQtAppsHook
          ];

          dontUseCmakeConfigure = true;
          dontUseNinjaBuild = true;
          dontUseNinjaInstall = true;

          buildInputs = with pkgs; [
            qtEnv
            pkgs.qt6.qtbase
            pkgs.qt6.qtdeclarative
            pkgs.qt6.qtsvg
            pkgs.qt6.qtwayland
            pkgs.libGL
            pkgs.vulkan-loader
            pkgs.libx11
            pkgs.libxkbcommon
            pkgs.ffmpeg
          ];

          LRELEASE = "${qtEnv}/bin/lrelease";
          QT_PLUGIN_PATH = "${qtEnv}/lib/qt-6/plugins";
          QML2_IMPORT_PATH = "${qtEnv}/lib/qt-6/qml";

          preBuild = ''
            mkdir -p $TMPDIR/qt-wrap
            cat > $TMPDIR/qt-wrap/qt.conf <<EOF
[Paths]
Prefix = ${qtEnv}
Headers = include
Libraries = lib
Plugins = lib/qt-6/plugins
QmlImports = lib/qt-6/qml
EOF
            ln -s ${qtEnv}/bin/qmake $TMPDIR/qt-wrap/qmake
            export QMAKE="$TMPDIR/qt-wrap/qmake"
            export PATH="$TMPDIR/qt-wrap:$PATH"
          '';

          cargoBuildFlags = [
            "-p" "nectarlink-desktop"
            "-p" "nectarlink-cli"
          ];

          doCheck = false;

          preFixup = ''
            patchelf --add-rpath "${pkgs.lib.makeLibraryPath (with pkgs; [
              libGL
              vulkan-loader
              libx11
              libxkbcommon
              ffmpeg
            ])}" $out/bin/nectarlink-desktop
          '';

          postInstall = ''
            # Translations
            mkdir -p $out/share/nectarlink
            for qm in target/*/*/nectarlink_*.qm target/*/nectarlink_*.qm; do
              if [ -f "$qm" ]; then
                cp -n "$qm" $out/share/nectarlink/ || true
              fi
            done

            # Desktop integration
            install -Dm644 desktop/packaging/linux/nectarlink.desktop $out/share/applications/nectarlink.desktop
            install -Dm644 desktop/packaging/linux/org.nectarlink.desktop.metainfo.xml $out/share/metainfo/org.nectarlink.desktop.metainfo.xml

            # Icons
            if [ -f assets/brand/icon.svg ]; then
              install -Dm644 assets/brand/icon.svg $out/share/icons/hicolor/scalable/apps/nectarlink.svg
              install -Dm644 assets/brand/icon.svg $out/share/pixmaps/nectarlink.svg
            fi
            if [ -f assets/brand/icon-512.png ]; then
              install -Dm644 assets/brand/icon-512.png $out/share/icons/hicolor/512x512/apps/nectarlink.png
              install -Dm644 assets/brand/icon-512.png $out/share/pixmaps/nectarlink.png
            fi
          '';

          meta = with pkgs.lib; {
            description = "Your Android phone and your PC, working as one";
            homepage = "https://github.com/thtbee/nectarlink";
            license = licenses.gpl3Plus;
            maintainers = with maintainers; [ ];
            platforms = platforms.linux;
            mainProgram = "nectarlink-desktop";
          };
        };
    in
    (flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" ] (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        qtEnv = pkgs.qt6.env "qt6-env" [
          pkgs.qt6.qtbase
          pkgs.qt6.qtdeclarative
          pkgs.qt6.qtsvg
          pkgs.qt6.qtwayland
          pkgs.qt6.qtshadertools
          pkgs.qt6.qttools
        ];
      in {
        packages.default = packageBuilder system;
        packages.nectarlink = packageBuilder system;

        apps.default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/nectarlink-desktop";
          meta.description = "NectarLink desktop application";
        };

        apps.cli = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/nectarlink";
          meta.description = "NectarLink command-line client";
        };

        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            pkg-config
            cmake
            ninja
            rustc
            cargo
            rustfmt
            clippy
            rust-analyzer
            patchelf
          ];

          buildInputs = with pkgs; [
            qtEnv
            qt6.qtbase
            qt6.qtdeclarative
            qt6.qtsvg
            qt6.qtwayland
            libGL
            vulkan-loader
            libx11
            libxkbcommon
            ffmpeg
          ];

          shellHook = ''
            export QT_WRAP_DIR=$(mktemp -d)
            cat > "$QT_WRAP_DIR/qt.conf" <<EOF
[Paths]
Prefix = ${qtEnv}
Headers = include
Libraries = lib
Plugins = lib/qt-6/plugins
QmlImports = lib/qt-6/qml
EOF
            ln -s "${qtEnv}/bin/qmake" "$QT_WRAP_DIR/qmake"
            export QMAKE="$QT_WRAP_DIR/qmake"
            export PATH="$QT_WRAP_DIR:$PATH"
          '';

          LRELEASE = "${qtEnv}/bin/lrelease";
          QT_PLUGIN_PATH = "${qtEnv}/lib/qt-6/plugins";
          QML2_IMPORT_PATH = "${qtEnv}/lib/qt-6/qml";
          LD_LIBRARY_PATH = "${pkgs.lib.makeLibraryPath (with pkgs; [
            qtEnv
            libGL
            vulkan-loader
            libx11
            libxkbcommon
            ffmpeg
          ])}:$LD_LIBRARY_PATH";
          LIBRARY_PATH = "${pkgs.lib.makeLibraryPath (with pkgs; [
            qtEnv
            libGL
            vulkan-loader
            libx11
            libxkbcommon
            ffmpeg
          ])}:$LIBRARY_PATH";
        };
      })
    ) // {
      overlays.default = final: prev: {
        nectarlink = packageBuilder final.system;
      };
    };
}
