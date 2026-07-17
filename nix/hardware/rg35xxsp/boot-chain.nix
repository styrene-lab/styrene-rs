{ pkgs, lib ? pkgs.lib }:
let
  tfARevision = "aa1793fff49a1b5a6a877c278a0df0a188e2b1f2";
  uBootRevision = "88dc2788777babfd6322fa655df549a019aa1e69";
  linuxRevision = "9021cc14f7d98b4a1d2c932f52c5343d4d0f6b92";

  tfASource = pkgs.fetchFromGitHub {
    owner = "ARM-software";
    repo = "arm-trusted-firmware";
    rev = tfARevision;
    hash = "sha256-aJx4E+YWaadruRtN45tIjaZKkGTiuCo7NqC6OL1QIdU=";
  };

  uBootSource = pkgs.fetchFromGitHub {
    owner = "u-boot";
    repo = "u-boot";
    rev = uBootRevision;
    hash = "sha256-LobC22bYpHVGZd5G8IugfcmHacVaHH0aNe3zQG7LJv0=";
  };

  linuxSource = pkgs.fetchFromGitHub {
    owner = "gregkh";
    repo = "linux";
    rev = linuxRevision;
    hash = "sha256-eSU5Ww3RuaZOC5m6KQ7AiW/VnHTkoQKu8cB9n9mcHYY=";
  };

  tfA = pkgs.stdenv.mkDerivation {
    pname = "rg35xxsp-trusted-firmware-a";
    version = "2.14.2-${builtins.substring 0 12 tfARevision}";
    src = tfASource;
    nativeBuildInputs = [ pkgs.dtc ];
    enableParallelBuilding = true;
    makeFlags = [
      "PLAT=sun50i_h616"
      "ARCH=aarch64"
      "CROSS_COMPILE=${pkgs.stdenv.cc.targetPrefix}"
      "CC=${pkgs.stdenv.cc.targetPrefix}gcc"
      "AS=${pkgs.stdenv.cc.targetPrefix}gcc"
      "LD=${pkgs.stdenv.cc.targetPrefix}ld"
      "OBJCOPY=${pkgs.stdenv.cc.targetPrefix}objcopy"
      "bl31"
    ];
    installPhase = ''
      runHook preInstall
      install -Dm0644 build/sun50i_h616/release/bl31.bin $out/bl31.bin
      runHook postInstall
    '';
  };

  uBoot = pkgs.buildUBoot {
    pname = "rg35xxsp-u-boot";
    version = "2026.04-${builtins.substring 0 12 uBootRevision}";
    src = uBootSource;
    defconfig = "anbernic_rg35xx_h700_defconfig";
    BL31 = "${tfA}/bl31.bin";
    extraMakeFlags = [ "BL31=${tfA}/bl31.bin" ];
    extraMeta.platforms = [ "aarch64-linux" ];
    filesToInstall = [
      "u-boot-sunxi-with-spl.bin"
      "u-boot.bin"
      "u-boot.dtb"
      ".config"
    ];
  };

  kernel = pkgs.buildLinux {
    pname = "rg35xxsp-linux";
    version = "7.0.9-${builtins.substring 0 12 linuxRevision}";
    modDirVersion = "7.0.9";
    src = linuxSource;
    defconfig = "defconfig";
    autoModules = true;
    structuredExtraConfig = with lib.kernel; {
      ARCH_SUNXI = yes;
      DRM = yes;
      DRM_PANEL = yes;
      INPUT_EVDEV = yes;
      INPUT_GPIO_ROTARY_ENCODER = module;
      KEYBOARD_GPIO = yes;
      RTC_DRV_PCF8563 = module;
      WLAN = yes;
    };
    extraMeta.platforms = [ "aarch64-linux" ];
  };

  bundle = pkgs.runCommand "rg35xxsp-boot-chain-bundle" {
    nativeBuildInputs = [ pkgs.coreutils ];
  } ''
    mkdir -p $out/boot $out/provenance
    install -m0644 ${tfA}/bl31.bin $out/boot/bl31.bin
    install -m0644 ${uBoot}/u-boot-sunxi-with-spl.bin $out/boot/u-boot-sunxi-with-spl.bin
    install -m0644 ${kernel}/Image $out/boot/Image
    dtb=$(find ${kernel}/dtbs -name 'sun50i-h700-anbernic-rg35xx-sp.dtb' -print -quit)
    test -n "$dtb"
    install -m0644 "$dtb" $out/boot/sun50i-h700-anbernic-rg35xx-sp.dtb
    cat > $out/provenance/revisions <<EOF
    trusted-firmware-a=$tfARevision
    u-boot=$uBootRevision
    linux=$linuxRevision
    EOF
    (cd $out && sha256sum boot/* > provenance/sha256sums)
  '';
in {
  inherit tfA uBoot kernel bundle;
}
