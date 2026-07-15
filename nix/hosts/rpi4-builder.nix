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

  networking.firewall.allowedTCPPorts = [ 22 ];

  assertions = [{
    assertion = builtins.getEnv "STYRENE_BUILDER_SSH_KEY" != "";
    message = "Set STYRENE_BUILDER_SSH_KEY to the operator public key while evaluating the builder image";
  }];
}
