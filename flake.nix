{
  description = "Antarctical: a native GPUI Google Calendar viewer for Linux";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      buildInputs =
        pkgs: with pkgs; [
          fontconfig
          freetype
          libxcb
          libxkbcommon
          vulkan-loader
          vulkan-headers
          wayland
        ];

      # GPUI dlopens Vulkan, Wayland and xkbcommon at runtime.
      runtimeLibraryPath =
        pkgs:
        pkgs.lib.makeLibraryPath (
          with pkgs;
          [
            libxkbcommon
            vulkan-loader
            wayland
          ]
        );
    in
    {
      packages = forAllSystems (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "antarctical";
          version = "0.1.0";
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              ./src
              ./data
            ];
          };

          cargoLock = {
            lockFile = ./Cargo.lock;
            allowBuiltinFetchGit = true;
          };

          nativeBuildInputs = with pkgs; [
            makeWrapper
            pkg-config
          ];
          buildInputs = buildInputs pkgs;

          postInstall = ''
            install -Dm644 data/com.korora.Antarctical.desktop $out/share/applications/com.korora.Antarctical.desktop
          '';

          postFixup = ''
            wrapProgram $out/bin/antarctical \
              --prefix LD_LIBRARY_PATH : ${runtimeLibraryPath pkgs}
          '';

          meta = {
            description = "A native GPUI Google Calendar viewer for Linux";
            license = pkgs.lib.licenses.asl20;
            mainProgram = "antarctical";
            platforms = systems;
          };
        };
      });

      apps = forAllSystems (pkgs: {
        default = {
          type = "app";
          program = nixpkgs.lib.getExe self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        };
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [ self.packages.${pkgs.stdenv.hostPlatform.system}.default ];
          packages = with pkgs; [
            cargo
            clippy
            rust-analyzer
            rustc
            rustfmt
          ];
          LD_LIBRARY_PATH = runtimeLibraryPath pkgs;
        };
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt);
    };
}
