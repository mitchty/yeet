{
  config,
  lib,
  pkgs,
  ...
}:

with lib;

let
  cfg = config.services.yeet;
in
{
  options.services.yeet = {
    enable = mkEnableOption "yeet daemon";

    package = mkOption {
      type = types.package;
      default = pkgs.yeet;
      defaultText = literalExpression "pkgs.yeet";
      description = mdDoc "The yeet package to use for the daemon.";
    };
  };

  config = mkIf cfg.enable {
    systemd.services.yeet = {
      description = "Yeet file sync daemon";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];

      serviceConfig = {
        Type = "simple";
        ExecStart = "${cfg.package}/bin/yeet serve";
        Restart = "on-failure";
        RestartSec = "5s";
      };
    };
  };
}
