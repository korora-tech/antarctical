# Home Manager module: `programs.antarctical`. Takes this flake (`self`) for its packages.
self:
{
  config,
  lib,
  options,
  pkgs,
  ...
}:
let
  cfg = config.programs.antarctical;
  inherit (pkgs.stdenv.hostPlatform) system;
  # Only touch Dux's options when its Home Manager module is imported.
  hasDux = options ? programs.dank-material-shell;
in
{
  options.programs.antarctical = {
    enable = lib.mkEnableOption "Antarctical, a native Google Calendar viewer";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${system}.default;
      defaultText = lib.literalExpression "antarctical.packages.\${system}.default";
      description = "The Antarctical package.";
    };

    service.enable = lib.mkEnableOption ''
      a systemd user service that starts Antarctical in the tray (`--hidden`) with the
      graphical session'';

    noctalia.enable = lib.mkEnableOption ''
      the Noctalia plugin, linked into Noctalia's local plugin directory as
      `korora-tech/antarctical` (enable it in Noctalia and add its
      `korora-tech/antarctical:bar` widget to a bar yourself)'';

    vdir.enable = lib.mkEnableOption ''
      a systemd user service that mirrors the calendars shown in Antarctical's sidebar into a
      vdir (`antarctical vdir`), for calendars that read one, such as Noctalia's'';

    vdir.path = lib.mkOption {
      type = lib.types.str;
      default = "${config.xdg.dataHome}/antarctical/calendars";
      defaultText = lib.literalExpression ''"''${config.xdg.dataHome}/antarctical/calendars"'';
      description = "The directory `vdir.enable` mirrors the calendars into.";
    };

    dux.enable = lib.mkEnableOption ''
      the Dux shell integration: start Antarctical before Dux (`dms.service`) so Dux finds its
      dankcal socket'';

    dux.setCalendarBackend = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Set Dux's `calendarBackend` to `dankcal` through
        `programs.dank-material-shell.settings` (needs Dux's Home Manager module and
        `dux.enable`). This makes Home Manager own `~/.config/dms/settings.json`: it becomes
        a read-only link holding only the settings declared in Nix, so settings changed in
        Dux's GUI are lost. Leave it off and pick the backend in Dux unless you already
        manage Dux's settings with Home Manager.
      '';
    };
  };

  config = lib.mkIf cfg.enable (
    lib.mkMerge [
      {
        home.packages = [ cfg.package ];
        assertions = [
          {
            assertion = cfg.dux.enable -> cfg.service.enable;
            message = "programs.antarctical.dux.enable requires programs.antarctical.service.enable.";
          }
          {
            assertion = cfg.dux.setCalendarBackend -> cfg.dux.enable && hasDux;
            message = "programs.antarctical.dux.setCalendarBackend requires dux.enable and Dux's Home Manager module.";
          }
        ];
      }

      (lib.mkIf cfg.service.enable {
        systemd.user.services.antarctical = {
          Unit = {
            Description = "Antarctical Google Calendar viewer";
            After = [ "graphical-session.target" ];
            PartOf = [ "graphical-session.target" ];
          };
          # `always`: a launch that finds Antarctical already running (started by hand) logs
          # one line and exits 0; retrying lets the service take over once that one quits.
          # Quitting from the tray therefore only lasts until the next retry; stop the unit to
          # keep it closed.
          Service = {
            ExecStart = "${lib.getExe cfg.package} --hidden";
            Restart = "always";
            RestartSec = 10;
          };
          Install.WantedBy = [ "graphical-session.target" ];
        };
      })

      (lib.mkIf cfg.noctalia.enable {
        xdg.dataFile."noctalia/plugins/antarctical".source = pkgs.callPackage ./noctalia-plugin.nix {
          antarctical = cfg.package;
        };
      })

      (lib.mkIf cfg.vdir.enable {
        systemd.user.services.antarctical-vdir = {
          Unit = {
            Description = "Antarctical calendars as a vdir";
            After = [ "graphical-session.target" ];
            PartOf = [ "graphical-session.target" ];
          };
          # The mirror waits for Antarctical by itself; it only exits when the directory
          # cannot be written.
          Service = {
            ExecStart = "${lib.getExe cfg.package} vdir ${lib.escapeShellArg cfg.vdir.path}";
            Restart = "on-failure";
            RestartSec = 30;
          };
          Install.WantedBy = [ "graphical-session.target" ];
        };
      })

      # Dux looks for the socket at start and then only for about a minute.
      (lib.mkIf cfg.dux.enable {
        systemd.user.services.antarctical.Unit.Before = [ "dms.service" ];
      })

      (lib.optionalAttrs hasDux {
        programs.dank-material-shell = lib.mkIf (cfg.dux.enable && cfg.dux.setCalendarBackend) {
          settings.calendarBackend = "dankcal";
        };
      })
    ]
  );
}
