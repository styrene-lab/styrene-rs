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
  bootChain,
  imageName ? "styrene-rg35xxsp-bringup-image",
  imageBaseName ? "styrene-rg35xxsp-bringup",
  compressImage ? true,
  bootSizeMiB ? 256,
  rootOffsetMiB ? 264,
}:

assert rootOffsetMiB > bootSizeMiB;
stdenv.mkDerivation {
  name = imageName;
  nativeBuildInputs = [ dosfstools e2fsprogs libfaketime mtools util-linux ]
    ++ lib.optional compressImage zstd;
  dontUnpack = true;

  buildPhase = ''
    runHook preBuild
    work="$NIX_BUILD_TOP/rg35xxsp-image"
    mkdir -p "$work/boot"
    cd "$work"

    root_fs=${rootFilesystemImage}
    ${lib.optionalString compressImage ''
      root_fs="$work/root-fs.img"
      zstd -d --no-progress ${rootFilesystemImage} -o "$root_fs"
    ''}

    cp ${bootChain}/boot/Image boot/Image
    cp ${bootChain}/boot/sun50i-h700-anbernic-rg35xx-sp.dtb boot/
    cp ${bootChain}/provenance/revisions boot/boot-chain-revisions
    cp ${bootChain}/provenance/sha256sums boot/boot-chain-sha256sums
    mkdir -p boot/extlinux
    cat > boot/extlinux/extlinux.conf <<'EOF'
    DEFAULT nixos
    TIMEOUT 30
    MENU TITLE Styrene RG35XXSP bring-up

    LABEL nixos
      LINUX /Image
      FDT /sun50i-h700-anbernic-rg35xx-sp.dtb
      APPEND console=ttyS0,115200 console=tty0 loglevel=7 root=LABEL=NIXOS_ROOT rootwait rw
    EOF

    root_blocks=$(du -B 512 --apparent-size "$root_fs" | awk '{print $1}')
    root_start=$(( ${toString rootOffsetMiB} * 1024 * 1024 / 512 ))
    boot_start=$(( 8 * 1024 * 1024 / 512 ))
    boot_sectors=$(( ${toString bootSizeMiB} * 1024 * 1024 / 512 ))
    image_size=$((root_start * 512 + root_blocks * 512))
    img="$work/${imageBaseName}.img"
    truncate -s "$image_size" "$img"

    sfdisk --no-reread --no-tell-kernel "$img" <<EOF
    label: dos
    label-id: 0x3500a11e

    start=$boot_start, size=$boot_sectors, type=c, bootable
    start=$root_start, type=83
    EOF

    # Allwinner BootROM reads the combined SPL/U-Boot image from 8 KiB. Keep
    # this raw region before the first partition and assert its bounded size.
    uboot=${bootChain}/boot/u-boot-sunxi-with-spl.bin
    test "$(stat -c %s "$uboot")" -le $((boot_start * 512 - 8192))
    dd if="$uboot" of="$img" bs=1024 seek=8 conv=notrunc status=none

    truncate -s $((boot_sectors * 512)) boot.img
    mkfs.vfat --invariant -i 3500A11E -n STYRENE_BOOT boot.img
    cd boot
    while IFS= read -r d; do faketime '2000-01-01' mmd -i ../boot.img "::/$d"; done < <(find . -mindepth 1 -type d -printf '%P\n' | sort)
    while IFS= read -r f; do mcopy -pvm -i ../boot.img "$f" "::/$f"; done < <(find . -type f -printf '%P\n' | sort)
    cd ..
    fsck.vfat -vn boot.img
    dd if=boot.img of="$img" bs=512 seek=$boot_start conv=notrunc status=none
    dd if="$root_fs" of="$img" bs=512 seek=$root_start conv=notrunc status=none

    mkdir manifest
    cp ${bootChain}/provenance/revisions manifest/boot-chain-revisions
    cp ${bootChain}/provenance/sha256sums manifest/boot-chain-sha256sums
    cat > manifest/layout <<EOF
    schema_version=1
    delivery_authorized=false
    bootloader_offset_bytes=8192
    boot_partition_offset_bytes=$((boot_start * 512))
    boot_partition_size_bytes=$((boot_sectors * 512))
    root_partition_offset_bytes=$((root_start * 512))
    root_label=NIXOS_ROOT
    EOF
    sha256sum "$img" > manifest/image.sha256
    cp -r manifest "$work/"

    ${lib.optionalString compressImage ''zstd -T$NIX_BUILD_CORES --rm "$img"''}
    runHook postBuild
  '';

  installPhase = ''
    mkdir -p "$out/sd-image" "$out/manifest" "$out/nix-support"
    cp "$NIX_BUILD_TOP/rg35xxsp-image/${imageBaseName}.img"* "$out/sd-image/"
    cp -r "$NIX_BUILD_TOP/rg35xxsp-image/manifest/." "$out/manifest/"
    echo "file sd-image $out/sd-image/${imageBaseName}.img${lib.optionalString compressImage ".zst"}" > "$out/nix-support/hydra-build-products"
  '';
}
