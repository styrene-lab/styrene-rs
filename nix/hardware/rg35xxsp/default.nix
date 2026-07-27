# RG35XXSP board module skeleton. Boot-critical inputs remain deliberately
# unresolved until WP1 pins source revisions, patches, hashes, and licenses.
{ lib, ... }:
{
  assertions = [
    {
      assertion = false;
      message = ''
        RG35XXSP board support is not pinned yet. Resolve U-Boot/TF-A, kernel,
        sun50i-h700-anbernic-rg35xx-sp DTB, firmware, and image boot offsets in
        nix/hardware/rg35xxsp/provenance.toml before enabling this module.
      '';
    }
  ];

  # Expected final shape, intentionally inactive:
  # hardware.deviceTree.name =
  #   "allwinner/sun50i-h700-anbernic-rg35xx-sp.dtb";
  # boot.loader.generic-extlinux-compatible.enable = true;
  # boot.kernelPackages = pkgs.linuxPackagesFor rg35xxspKernel;
}
