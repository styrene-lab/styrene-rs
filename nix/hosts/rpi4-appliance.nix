{ lib, styrenePackage, ... }:
{
  networking.hostName = lib.mkForce "styrene-appliance-b";

  services.styrene-qemu-smoke = {
    enable = true;
    package = styrenePackage;
  };

  # Temporary bring-up access. Remove once serial and product control paths are proven.
  services.openssh = {
    enable = lib.mkForce true;
    settings = {
      PasswordAuthentication = false;
      KbdInteractiveAuthentication = false;
      PermitRootLogin = "no";
    };
  };
  users.users.styrene-admin = {
    isNormalUser = true;
    description = "Styrene appliance bring-up operator";
    extraGroups = [ "wheel" ];
    openssh.authorizedKeys.keys = lib.optionals
      (builtins.getEnv "STYRENE_APPLIANCE_SSH_KEY" != "")
      [ (builtins.getEnv "STYRENE_APPLIANCE_SSH_KEY") ];
  };
  security.sudo.wheelNeedsPassword = false;
  networking.firewall.allowedTCPPorts = [ 22 ];

  assertions = [{
    assertion = builtins.getEnv "STYRENE_APPLIANCE_SSH_KEY" != "";
    message = "Set STYRENE_APPLIANCE_SSH_KEY to the operator public key while evaluating the appliance image";
  }];
}
