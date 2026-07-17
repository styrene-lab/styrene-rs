{
  config,
  lib,
  pkgs,
  ...
}:
let
  firmwareTree = pkgs.runCommand "rpi4-firmware-tree" { } ''
    mkdir -p "$out/overlays"
    cp ${pkgs.raspberrypi-armstubs}/armstub8-gic.bin "$out/armstub8-gic.bin"
    cp ${pkgs.ubootRaspberryPi4_64bit}/u-boot.bin "$out/u-boot-rpi4.bin"
    cp ${pkgs.raspberrypifw}/share/raspberrypi/boot/bootcode.bin "$out/"
    cp ${pkgs.raspberrypifw}/share/raspberrypi/boot/fixup*.dat "$out/"
    cp ${pkgs.raspberrypifw}/share/raspberrypi/boot/start*.elf "$out/"
    cp ${pkgs.raspberrypifw}/share/raspberrypi/boot/bcm2711-rpi-4-b.dtb "$out/"
    cat > "$out/config.txt" <<'EOF'
    arm_64bit=1
    enable_uart=1
    avoid_warnings=1
    kernel=u-boot-rpi4.bin
    EOF
  '';

  image = config.image;
  customImage = pkgs.callPackage ../make-rpi-sd-image.nix {
    rootFilesystemImage = config.sdImage.rootFilesystemImage;
    inherit firmwareTree;
    imageName = image.fileName;
    imageBaseName = image.baseName;
    buildPlatform = pkgs.stdenv.buildPlatform.system;
    inherit (config.sdImage)
      compressImage
      firmwareSize
      firmwarePartitionID
      firmwarePartitionName
      postBuildCommands
      ;
    firmwareOffset = config.sdImage.firmwarePartitionOffset;
  };
in
{
  # Own the final image derivation rather than patching the upstream shell hook.
  # This keeps all mutable staging below NIX_BUILD_TOP and avoids macOS virtiofs
  # ACL/xattr leakage into image construction.
  system.build.sdImage = lib.mkForce customImage;
  system.build.image = lib.mkForce customImage;
}
