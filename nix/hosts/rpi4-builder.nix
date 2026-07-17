{ lib, ... }:
{
  networking.hostName = "styrene-builder-a";

  services.openssh = {
    enable = true;
    settings = {
      PasswordAuthentication = false;
      KbdInteractiveAuthentication = false;
      PermitRootLogin = "no";
    };
  };

  users.users.nix-builder = {
    isNormalUser = true;
    description = "Restricted Nix remote builder";
    group = "nix-builder";
    openssh.authorizedKeys.keys = lib.optionals
      (builtins.getEnv "STYRENE_BUILDER_SSH_KEY" != "")
      [ (builtins.getEnv "STYRENE_BUILDER_SSH_KEY") ];
  };
  users.groups.nix-builder = { };
  nix.settings.trusted-users = [ "root" "nix-builder" ];
  nix.settings.max-jobs = 1;
  nix.settings.cores = 3;

  systemd.services.styrene-builder-state = {
    description = "Prepare writable Nix builder runtime state";
    wantedBy = [ "multi-user.target" ];
    before = [ "nix-daemon.socket" "nix-daemon.service" ];
    unitConfig.DefaultDependencies = false;
    after = [ "local-fs.target" ];
    serviceConfig.Type = "oneshot";
    serviceConfig.RemainAfterExit = true;
    script = ''
      mkdir -p /nix/var/nix/daemon-socket /nix/var/nix/db /nix/var/nix/gcroots /nix/var/nix/profiles /nix/var/nix/temproots /nix/var/nix/userpool
      chmod 0755 /nix/var /nix/var/nix
      chmod 0775 /nix/var/nix/daemon-socket
      chown root:nixbld /nix/var/nix/daemon-socket
    '';
  };

  systemd.sockets.nix-daemon.wants = [ "styrene-builder-state.service" ];
  systemd.services.nix-daemon.wants = [ "styrene-builder-state.service" ];

  services.logrotate.enable = false;

  networking.firewall.allowedTCPPorts = [ 22 ];

  assertions = [{
    assertion = builtins.getEnv "STYRENE_BUILDER_SSH_KEY" != "";
    message = "Set STYRENE_BUILDER_SSH_KEY to the operator public key while evaluating the builder image";
  }];
}
