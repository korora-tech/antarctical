{
  description = "Antarctical: a native GPUI Google Calendar viewer for Linux";

  nixConfig = {
    extra-substituters = [ "https://korora-tech.cachix.org" ];
    extra-trusted-public-keys = [
      "korora-tech.cachix.org-1:42bs8TVW3uvxvMnuJtkXP5lkHFK7++Sral9uRDautuA="
    ];
  };

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
              ./LICENSE
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
            install -Dm644 LICENSE $out/share/licenses/antarctical/LICENSE
            install -Dm644 data/korora.tech.Antarctical.desktop $out/share/applications/korora.tech.Antarctical.desktop
            install -Dm644 data/branding/antarctical-app-icon.svg $out/share/icons/hicolor/scalable/apps/korora.tech.Antarctical.svg
            install -Dm644 data/branding/antarctical-app-icon.png $out/share/icons/hicolor/512x512/apps/korora.tech.Antarctical.png
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

        noctalia-plugin = pkgs.callPackage ./nix/noctalia-plugin.nix {
          antarctical = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        };
      });

      homeModules = {
        default = self.homeModules.antarctical;
        antarctical = import ./nix/hm-module.nix self;
      };

      # Evaluates the Home Manager module against stub options, with and without Dux's module.
      checks = forAllSystems (
        pkgs:
        let
          inherit (nixpkgs) lib;
          system = pkgs.stdenv.hostPlatform.system;
          hmStub = {
            options = {
              home.packages = lib.mkOption {
                type = lib.types.listOf lib.types.package;
                default = [ ];
              };
              assertions = lib.mkOption {
                type = lib.types.listOf lib.types.attrs;
                default = [ ];
              };
              systemd.user.services = lib.mkOption {
                type = lib.types.attrsOf (lib.types.attrsOf lib.types.anything);
                default = { };
              };
              xdg.dataHome = lib.mkOption {
                type = lib.types.str;
                default = "/home/me/.local/share";
              };
              xdg.dataFile = lib.mkOption {
                type = lib.types.attrsOf (lib.types.attrsOf lib.types.anything);
                default = { };
              };
            };
          };
          duxStub = {
            options.programs.dank-material-shell = {
              settings = lib.mkOption {
                type = lib.types.attrsOf lib.types.anything;
                default = { };
              };
            };
          };
          eval =
            modules: settings:
            (lib.evalModules {
              modules = [
                hmStub
                self.homeModules.default
                {
                  _module.args.pkgs = pkgs;
                  programs.antarctical = settings;
                }
              ]
              ++ modules;
            }).config;
          all = {
            enable = true;
            service.enable = true;
            dux.enable = true;
          };
          withDux = eval [ duxStub ] all;
          withDuxBackend = eval [ duxStub ] (lib.recursiveUpdate all { dux.setCalendarBackend = true; });
          withoutDux = eval [ ] all;
          withNoctalia = eval [ ] {
            enable = true;
            noctalia.enable = true;
            vdir.enable = true;
          };
          noService = eval [ ] {
            enable = true;
            dux.enable = true;
          };
          backendWithoutDux = eval [ ] (lib.recursiveUpdate all { dux.setCalendarBackend = true; });
          unit = c: c.systemd.user.services.antarctical;
          holds = c: lib.all (a: a.assertion) c.assertions;
          checks = [
            (lib.assertMsg (
              (unit withDux).Service.ExecStart == "${lib.getExe self.packages.${system}.default} --hidden"
            ) "service runs antarctical --hidden")
            (lib.assertMsg ((unit withDux).Service.Restart == "always") "service takes over after a hand-off")
            (lib.assertMsg ((unit withDux).Unit.Before == [ "dms.service" ]) "service starts before Dux")
            (lib.assertMsg (
              withDux.programs.dank-material-shell.settings == { }
            ) "Dux settings.json is left alone by default")
            (lib.assertMsg (
              withDuxBackend.programs.dank-material-shell.settings.calendarBackend == "dankcal"
              && holds withDuxBackend
            ) "setCalendarBackend sets the dankcal backend")
            (lib.assertMsg (
              (unit withoutDux).Unit.Before == [ "dms.service" ] && !(withoutDux.programs ? dank-material-shell)
            ) "works without Dux's module")
            (lib.assertMsg (holds withDux && holds withoutDux) "assertions hold")
            (lib.assertMsg (!(holds noService)) "dux.enable without service.enable is refused")
            (lib.assertMsg (!(holds backendWithoutDux)) "setCalendarBackend without Dux's module is refused")
            (lib.assertMsg (
              withoutDux.home.packages == [ self.packages.${system}.default ]
            ) "installs the package")
            (lib.assertMsg (
              withoutDux.xdg.dataFile == { } && !(withoutDux.systemd.user.services ? antarctical-vdir)
            ) "the Noctalia plugin and the vdir mirror are opt-in")
            (lib.assertMsg (
              withNoctalia.xdg.dataFile."noctalia/plugins/antarctical".source.outPath
              == self.packages.${system}.noctalia-plugin.outPath
            ) "Noctalia finds the plugin in its local plugin directory")
            (lib.assertMsg (
              withNoctalia.systemd.user.services.antarctical-vdir.Service.ExecStart
              == "${lib.getExe self.packages.${system}.default} vdir /home/me/.local/share/antarctical/calendars"
            ) "the vdir mirror writes under the data directory")
          ];
        in
        {
          hm-module = pkgs.runCommand "antarctical-hm-module-check" { } ''
            echo ${toString (lib.all lib.id checks)} > $out
          '';
        }
      );

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
