{ ... }:
{
  # virtiofs may expose unreadable synthetic security xattrs. The custom
  # creator omits host xattrs while retaining Nix store contents and modes.
  sdImage.rootFilesystemCreator = ../make-ext4-fs-no-host-xattrs.nix;
}
