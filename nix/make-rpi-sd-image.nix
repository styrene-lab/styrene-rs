{
  stdenv,
  lib,
  dosfstools,
  e2fsprogs,
  libfaketime,
  mtools,
  util-linux,
  zstd,
  rootFilesystemImage,
  firmwareTree,
  imageName,
  imageBaseName,
  buildPlatform,
  compressImage ? true,
  firmwareSize ? 256,
  firmwareOffset ? 8,
  firmwarePartitionID ? "2178694e",
  firmwarePartitionName ? "FIRMWARE",
  postBuildCommands ? "",
}:

stdenv.mkDerivation {
  name = imageName;

  nativeBuildInputs = [
    dosfstools
    e2fsprogs
    libfaketime
    mtools
    util-linux
  ] ++ lib.optional compressImage zstd;

  dontUnpack = true;

  buildPhase = ''
    runHook preBuild

    # Every mutable path is rooted in the Nix-owned build directory.  The
    # repository bind mount and derivation output are never used as scratch.
    work="$NIX_BUILD_TOP/styrene-sd-image"
    rm -rf "$work"
    mkdir -p "$work/firmware"
    cd "$work"

    root_fs=${rootFilesystemImage}
    ${lib.optionalString compressImage ''
      root_fs="$work/root-fs.img"
      zstd -d --no-progress ${rootFilesystemImage} -o "$root_fs"
    ''}

    cp -a --no-preserve=xattr ${firmwareTree}/. "$work/firmware/"
    find "$work/firmware" -exec touch --date=2000-01-01 {} +

    img="$work/${imageBaseName}.img"
    gap=${toString firmwareOffset}
    root_blocks=$(du -B 512 --apparent-size "$root_fs" | awk '{ print $1 }')
    firmware_blocks=$((${toString firmwareSize} * 1024 * 1024 / 512))
    image_size=$((root_blocks * 512 + firmware_blocks * 512 + gap * 1024 * 1024))
    truncate -s "$image_size" "$img"

    sfdisk --no-reread --no-tell-kernel "$img" <<EOF
        label: dos
        label-id: 0x${firmwarePartitionID}

        start=''${gap}M, size=$firmware_blocks, type=b
        start=$((gap + ${toString firmwareSize}))M, type=83, bootable
    EOF

    eval $(partx "$img" -o START,SECTORS --nr 2 --pairs)
    dd conv=notrunc if="$root_fs" of="$img" seek="$START" count="$SECTORS"

    eval $(partx "$img" -o START,SECTORS --nr 1 --pairs)
    truncate -s $((SECTORS * 512)) firmware-part.img
    mkfs.vfat --invariant -i ${firmwarePartitionID} -n ${firmwarePartitionName} firmware-part.img

    cd "$work/firmware"
    while IFS= read -r d; do
      faketime "2000-01-01 00:00:00" mmd -i "$work/firmware-part.img" "::/$d"
    done < <(find . -type d -mindepth 1 -printf '%P\n' | sort)
    while IFS= read -r f; do
      mcopy -pvm -i "$work/firmware-part.img" "$f" "::/$f"
    done < <(find . -type f -printf '%P\n' | sort)
    cd "$work"

    fsck.vfat -vn firmware-part.img
    dd conv=notrunc if=firmware-part.img of="$img" seek="$START" count="$SECTORS"
    ${postBuildCommands}

    ${lib.optionalString compressImage ''
      zstd -T$NIX_BUILD_CORES --rm "$img"
    ''}

    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    mkdir -p "$out/nix-support" "$out/sd-image"
    echo ${lib.escapeShellArg buildPlatform} > "$out/nix-support/system"
    ${if compressImage then ''
      cp "$NIX_BUILD_TOP/styrene-sd-image/${imageBaseName}.img.zst" "$out/sd-image/"
      echo "file sd-image $out/sd-image/${imageBaseName}.img.zst" > "$out/nix-support/hydra-build-products"
    '' else ''
      cp "$NIX_BUILD_TOP/styrene-sd-image/${imageBaseName}.img" "$out/sd-image/"
      echo "file sd-image $out/sd-image/${imageBaseName}.img" > "$out/nix-support/hydra-build-products"
    ''}
    runHook postInstall
  '';
}
