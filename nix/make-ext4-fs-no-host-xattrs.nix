{ pkgs
, lib
, storePaths
, compressImage ? false
, zstd
, populateImageCommands ? ""
, volumeLabel
, uuid ? "44444444-4444-4444-8888-888888888888"
, e2fsprogs
, libfaketime
, perl
, fakeroot
}:
let
  closureInfo = pkgs.buildPackages.closureInfo { rootPaths = storePaths; };
in
pkgs.stdenv.mkDerivation {
  name = "ext4-fs.img${lib.optionalString compressImage ".zst"}";
  nativeBuildInputs = [ e2fsprogs.bin libfaketime perl fakeroot pkgs.buildPackages.gnutar ]
    ++ lib.optional compressImage zstd;
  buildCommand = ''
    ${if compressImage then "img=temp.img" else "img=$out"}
    mkdir -p ./files
    ${populateImageCommands}

    mkdir -p ./rootImage/nix/store
    xargs -I % cp -a --no-preserve=xattr --reflink=auto % -t ./rootImage/nix/store/ \
      < ${closureInfo}/store-paths
    (
      GLOBIGNORE=".:.."
      shopt -u dotglob
      for f in ./files/*; do
        cp -a --no-preserve=xattr --reflink=auto -t ./rootImage/ "$f"
      done
    )
    cp --no-preserve=xattr ${closureInfo}/registration ./rootImage/nix-path-registration

    mkdir -p ./sanitizedRoot
    tar --xattrs-exclude='*' --no-xattrs -C ./rootImage -cf - . \
      | tar --no-xattrs -C ./sanitizedRoot -xf -
    numInodes=$(find ./sanitizedRoot | wc -l)
    numDataBlocks=$(du -s -c -B 4096 --apparent-size ./sanitizedRoot | tail -1 | awk '{ print int($1 * 1.20) }')
    bytes=$((2 * 4096 * numInodes + 4096 * numDataBlocks))
    mebibyte=$((1024 * 1024))
    if (( bytes % mebibyte )); then
      bytes=$(((bytes / mebibyte + 1) * mebibyte))
    fi
    echo "Creating an EXT4 image of $bytes bytes without host xattrs"
    truncate -s "$bytes" "$img"
    faketime -f "1970-01-01 00:00:01" fakeroot \
      mkfs.ext4 -L ${volumeLabel} -U ${uuid} "$img"
    ${pkgs.python3}/bin/python3 ${../scripts/rpi4_image.py} ./sanitizedRoot "$img"

    export EXT2FS_NO_MTAB_OK=yes
    # debugfs updates inode/block allocation but does not maintain all group
    # summary counters. Repair those deterministic metadata summaries before
    # performing the read-only verification gate.
    fsck.ext4 -y -f "$img"
    fsck.ext4 -n -f "$img"
    resize2fs -M "$img"
    sizeInBlocks=$(dumpe2fs -h "$img" 2>/dev/null | awk '/Block count:/ { print $3 }')
    blockSize=$(dumpe2fs -h "$img" 2>/dev/null | awk '/Block size:/ { print $3 }')
    truncate -s $((sizeInBlocks * blockSize)) "$img"
    ${lib.optionalString compressImage ''
      zstd -T$NIX_BUILD_CORES -v --no-progress "$img" -o "$out"
    ''}
  '';
}
