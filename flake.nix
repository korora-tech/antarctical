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

        # The Dux bar widget: link or copy to ~/.config/dms/plugins/antarcticalNextEvent
        # (the Home Manager module does this with `dux.enable`). Clicking it runs this flake's
        # Antarctical by store path, so it works whatever dms.service's PATH is.
        dux-plugin = pkgs.runCommand "antarctical-dux-plugin" { } ''
          mkdir -p $out
          cp ${./dux-plugin/plugin.json} $out/plugin.json
          substitute ${./dux-plugin/NextEvent.qml} $out/NextEvent.qml \
            --replace-fail "@antarctical@" "${
              pkgs.lib.getExe self.packages.${pkgs.stdenv.hostPlatform.system}.default
            }"
        '';
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
            };
          };
          duxStub = {
            options.programs.dank-material-shell = {
              settings = lib.mkOption {
                type = lib.types.attrsOf lib.types.anything;
                default = { };
              };
              plugins = lib.mkOption {
                type = lib.types.attrsOf (
                  lib.types.submodule { options.src = lib.mkOption { type = lib.types.package; }; }
                );
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
              withDux.programs.dank-material-shell.plugins.antarcticalNextEvent.src.outPath
              == self.packages.${system}.dux-plugin.outPath
            ) "Dux gets the widget")
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
