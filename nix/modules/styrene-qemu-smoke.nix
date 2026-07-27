{ config, lib, pkgs, ... }:

let
  cfg = config.services.styrene-qemu-smoke;
  smokeScript = pkgs.writeShellScript "styrene-qemu-smoke" ''
    set -eu
    install -d -m 0700 /state/evidence
    rm -f /state/evidence/qemu-smoke.pass

    ${cfg.package}/bin/styrene --version
    ${cfg.package}/bin/styrene doctor --root /state/doctor
    ${cfg.package}/bin/styrene ghost-check --root /state/ghost --timeout 15

    touch /state/evidence/qemu-smoke.pass
    echo STYRENE_QEMU_SMOKE=pass
  '';
in {
  options.services.styrene-qemu-smoke = {
    enable = lib.mkEnableOption "boot-time Styrene lifecycle evidence for the generic ARM64 VM";

    package = lib.mkOption {
      type = lib.types.package;
      description = "Package containing the canonical styrene executable";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];

    systemd.services.styrene-qemu-smoke = {
      description = "Styrene generic ARM64 VM lifecycle evidence";
      wantedBy = [ "multi-user.target" ];
      after = [ "local-fs.target" ];
      before = [ "getty.target" ];
      serviceConfig = {
        Type = "oneshot";
        ExecStart = smokeScript;
        RemainAfterExit = true;
        MemoryMax = "768M";
        TasksMax = 128;
        CPUQuota = "400%";
      };
    };
  };
}
