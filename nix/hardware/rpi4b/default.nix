# Shared Raspberry Pi 4B SD-image hardware composition.
{
  config,
  lib,
  modulesPath,
  pkgs,
  ...
}:
{
  imports = [ "${modulesPath}/installer/sd-card/sd-image-aarch64.nix" ];

  nixpkgs.hostPlatform = lib.mkDefault "aarch64-linux";
  boot.kernelPackages = lib.mkDefault pkgs.linuxKernel.packages.linux_rpi4;
  boot.loader.grub.enable = false;
  boot.loader.generic-extlinux-compatible.enable = true;
  boot.consoleLogLevel = 7;
  boot.kernelParams = [ "console=ttyS0,115200" "console=tty1" ];

  hardware.enableRedistributableFirmware = true;
  networking.useDHCP = lib.mkDefault true;

  sdImage = {
    compressImage = true;
    expandOnBoot = true;
    firmwareSize = 256;
    populateFirmwareCommands = let
      configTxt = pkgs.writeText "config.txt" ''
        arm_64bit=1
        enable_uart=1
        avoid_warnings=1
        kernel=u-boot-rpi4.bin
      '';
    in ''
      cp ${pkgs.raspberrypi-armstubs}/armstub8-gic.bin firmware/armstub8-gic.bin
      cp ${pkgs.ubootRaspberryPi4_64bit}/u-boot.bin firmware/u-boot-rpi4.bin
      cp ${pkgs.raspberrypifw}/share/raspberrypi/boot/bootcode.bin firmware/
      cp ${pkgs.raspberrypifw}/share/raspberrypi/boot/fixup*.dat firmware/
      cp ${pkgs.raspberrypifw}/share/raspberrypi/boot/start*.elf firmware/
      cp ${pkgs.raspberrypifw}/share/raspberrypi/boot/bcm2711-rpi-4-b.dtb firmware/
      cp ${configTxt} firmware/config.txt
    '';
  };
}
