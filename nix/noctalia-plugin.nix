# The Noctalia plugin directory, for ~/.local/share/noctalia/plugins/antarctical (the Home
# Manager module links it there with `noctalia.enable`). Its `command` setting defaults to
# `antarctical` by store path, so it works whatever PATH Noctalia was started with.
{
  lib,
  runCommand,
  antarctical,
}:
runCommand "antarctical-noctalia-plugin" { meta.license = lib.licenses.asl20; } ''
  cp -r ${../noctalia-plugin} $out
  chmod -R u+w $out
  cp ${../LICENSE} $out/LICENSE
  substituteInPlace $out/plugin.toml \
    --replace-fail 'default = "antarctical"' 'default = "${lib.getExe antarctical}"'
''
