# Shared Raspberry Pi 4B SD-image hardware composition.
{
  config,
  lib,
  modulesPath,
  pkgs,
  ...
}:
{
  imports = [
    "${modulesPath}/installer/sd-card/sd-image-aarch64.nix"
    ../../modules/sd-image-no-host-xattrs.nix
    ../../modules/rpi-sd-image.nix
  ];

  nixpkgs.hostPlatform = lib.mkDefault "aarch64-linux";
  boot.kernelPackages = lib.mkDefault pkgs.linuxKernel.packages.linux_rpi4;
  boot.loader.grub.enable = false;
  boot.loader.generic-extlinux-compatible.enable = true;
  boot.initrd.availableKernelModules = lib.mkForce [
    "mmc_block"
    "sdhci"
    "sdhci-iproc"
    "usbhid"
  ];
  boot.consoleLogLevel = 7;
  boot.kernelParams = [ "console=ttyS0,115200" "console=tty1" ];

  hardware.enableRedistributableFirmware = true;
  networking.useDHCP = lib.mkDefault true;

  sdImage = {
    compressImage = true;
    expandOnBoot = true;
    firmwareSize = 256;
    # The repository-owned image derivation assembles the boot tree.
    populateFirmwareCommands = lib.mkForce "";
  };
}
