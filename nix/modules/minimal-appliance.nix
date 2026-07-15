# Minimal NixOS profile shared by constrained handheld bring-up images.
{ config, lib, modulesPath, pkgs, ... }:
{
  imports = [ "${modulesPath}/profiles/minimal.nix" ];

  system.stateVersion = "26.05";
  nixpkgs.hostPlatform = lib.mkDefault "aarch64-linux";

  boot.loader.grub.enable = false;
  boot.loader.generic-extlinux-compatible.enable = true;
  boot.consoleLogLevel = 7;
  boot.kernelParams = [ "console=tty0" "loglevel=7" ];

  networking.hostName = "styrene-rg35xxsp";
  networking.useDHCP = false;
  networking.firewall.enable = true;

  documentation.enable = false;
  programs.command-not-found.enable = false;
  services.udisks2.enable = false;

  # Development image only. No password or network login is installed.
  services.openssh.enable = false;
  users.mutableUsers = false;
  users.allowNoPasswordLogin = true;

  environment.systemPackages = with pkgs; [
    coreutils
    util-linux
    iproute2
    kmod
  ];

  systemd.services.rg35xxsp-first-boot-evidence = {
    description = "Capture redacted RG35XXSP bring-up evidence";
    wantedBy = [ "multi-user.target" ];
    after = [ "systemd-udev-settle.service" "local-fs.target" ];
    wants = [ "systemd-udev-settle.service" ];
    serviceConfig = {
      Type = "oneshot";
      UMask = "0077";
    };
    script = ''
      set -eu
      out=/var/lib/rg35xxsp-bringup
      install -d -m 0700 "$out"
      report="$out/evidence.txt"
      {
        echo "schema_version=1"
        echo "system=${config.system.nixos.label or "unknown"}"
        echo "kernel=$(uname -r)"
        printf 'model='; tr -d '\000' </proc/device-tree/model 2>/dev/null || true; echo
        printf 'compatible='; tr '\000' ',' </proc/device-tree/compatible 2>/dev/null || true; echo
        echo "cmdline=$(cat /proc/cmdline)"
        echo "-- cpu --"; sed -n '1,80p' /proc/cpuinfo
        echo "-- memory --"; sed -n '1,40p' /proc/meminfo
        echo "-- mounts --"; findmnt --no-canonicalize
        echo "-- block --"; lsblk -o NAME,SIZE,TYPE,FSTYPE,LABEL,PARTLABEL,MOUNTPOINTS,RO
        echo "-- network --"; ip -brief link
        echo "-- input --"; sed -n '1,240p' /proc/bus/input/devices
        echo "-- drm --"; for f in /sys/class/drm/*/status; do test -r "$f" && echo "$f=$(cat "$f")"; done
        echo "-- framebuffer --"; for f in /sys/class/graphics/fb*/virtual_size; do test -r "$f" && echo "$f=$(cat "$f")"; done
        echo "-- power --"; for f in /sys/class/power_supply/*/{type,status,capacity}; do test -r "$f" && echo "$f=$(cat "$f")"; done
        echo "-- modules --"; cat /proc/modules
        echo "-- failed-units --"; systemctl --failed --no-legend || true
        echo "-- kernel-errors --"; journalctl -k -p warning..alert --no-pager -n 200 || true
      } >"$report"
      chmod 0600 "$report"
    '';
  };
}
