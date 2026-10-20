#!/bin/bash
# Build the Lulo OS live/installer ISO (ADR 0029).
#
#   sudo packaging/lulo-os-iso/build-iso.sh \
#       --version 0.9.0-beta.2 --debs /path/to/release-debs --out /path/to/out
#
# Runs on an Ubuntu 26.04 host (GitHub's ubuntu-26.04 runner) as root. Needs
# debootstrap, squashfs-tools, xorriso, mtools, dosfstools, librsvg2-bin and
# python3-pil. --debs holds rmac-apps, rmac-session, niri and
# xwayland-satellite for amd64, already checked against the release's
# SHA256SUMS. --work defaults to /mnt/lulo-iso (the runner's big scratch disk).
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
version=""
debs=""
out=""
work=/mnt/lulo-iso
mirror=http://archive.ubuntu.com/ubuntu
suite=resolute

while [ $# -gt 0 ]; do
    case "$1" in
        --version) version=$2; shift ;;
        --debs) debs=$2; shift ;;
        --out) out=$2; shift ;;
        --work) work=$2; shift ;;
        --mirror) mirror=$2; shift ;;
        *) echo "build-iso.sh: unknown argument $1" >&2; exit 2 ;;
    esac
    shift
done
[ -n "$version" ] && [ -d "$debs" ] && [ -n "$out" ] || {
    echo "usage: build-iso.sh --version V --debs DIR --out DIR [--work DIR]" >&2
    exit 2
}
[ "$(id -u)" -eq 0 ] || { echo "build-iso.sh: run as root" >&2; exit 1; }

chroot_dir=$work/chroot
image=$work/image
staging=$work/branding
iso_name="lulo-os-${version}-amd64.iso"
mkdir -p "$work" "$out"

cleanup() {
    for mount in dev/pts dev proc sys run; do
        mountpoint -q "$chroot_dir/$mount" && umount -l "$chroot_dir/$mount" || true
    done
}
trap cleanup EXIT

log() { echo "::group::$*"; }
endlog() { echo "::endgroup::"; }

# --- 1. Base system ----------------------------------------------------------
log "debootstrap $suite"
rm -rf "$chroot_dir" "$image"
debootstrap --arch=amd64 --variant=minbase \
    --components=main,restricted,universe,multiverse \
    "$suite" "$chroot_dir" "$mirror"
endlog

mount --bind /dev "$chroot_dir/dev"
mount --bind /dev/pts "$chroot_dir/dev/pts"
mount -t proc proc "$chroot_dir/proc"
mount -t sysfs sysfs "$chroot_dir/sys"
mount -t tmpfs tmpfs "$chroot_dir/run"

cat > "$chroot_dir/etc/apt/sources.list.d/ubuntu.sources" <<EOF
Types: deb
URIs: $mirror
Suites: $suite $suite-updates $suite-backports
Components: main restricted universe multiverse
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg

Types: deb
URIs: http://security.ubuntu.com/ubuntu
Suites: $suite-security
Components: main restricted universe multiverse
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg
EOF
rm -f "$chroot_dir/etc/apt/sources.list"
cp /etc/resolv.conf "$chroot_dir/etc/resolv.conf.build"

# --- 2. Branding (original Lulo artwork only, docs/brand.md) ----------------
log "branding"
rm -rf "$staging"
python3 "$here/make-branding.py" --repo "$repo" --out "$staging"
endlog

# --- 3. Packages, Lulo, overlay ----------------------------------------------
log "packages"
mkdir -p "$chroot_dir/tmp/lulo-build/debs"
cp "$debs"/*.deb "$chroot_dir/tmp/lulo-build/debs/"
cp "$here/packages.txt" "$here/chroot-setup.sh" "$chroot_dir/tmp/lulo-build/"
cp -a "$staging" "$chroot_dir/tmp/lulo-build/branding"
cp -a "$here/plymouth" "$chroot_dir/tmp/lulo-build/plymouth"
cp -a "$here/grub" "$chroot_dir/tmp/lulo-build/grub"
# The overlay goes in before the packages so policy files (casper.conf,
# netplan, GRUB defaults) are in place when their packages configure.
# Owned by root, and existing directories (/etc, /usr, ...) keep their own
# owner and mode.
tar -C "$here/rootfs" --owner=0 --group=0 -cf - . \
    | tar -C "$chroot_dir" --no-overwrite-dir -xf -
sed -i "s/@VERSION@/$version/g" "$chroot_dir/etc/calamares/branding/lulo/branding.desc"
chroot "$chroot_dir" /usr/bin/env -i \
    HOME=/root PATH=/usr/sbin:/usr/bin:/sbin:/bin LANG=C.UTF-8 \
    DEBIAN_FRONTEND=noninteractive LULO_VERSION="$version" \
    /bin/bash /tmp/lulo-build/chroot-setup.sh
endlog

# --- 4. Live medium -----------------------------------------------------------
log "medium"
mkdir -p "$image/casper" "$image/boot/grub" "$image/EFI/boot" "$image/.disk"
kernel=$(find "$chroot_dir/boot" -maxdepth 1 -name 'vmlinuz-*' | sort -V | tail -n 1)
initrd=$(find "$chroot_dir/boot" -maxdepth 1 -name 'initrd.img-*' | sort -V | tail -n 1)
cp "$kernel" "$image/casper/vmlinuz"
cp "$initrd" "$image/casper/initrd"

chroot "$chroot_dir" dpkg-query -W --showformat='${Package} ${Version}\n' \
    > "$image/casper/filesystem.manifest"
cp "$image/casper/filesystem.manifest" "$out/${iso_name%.iso}.manifest"
# Packages Calamares removes from the installed system.
printf '%s\n' calamares casper > "$image/casper/filesystem.manifest-remove"

cleanup
rm -rf "$chroot_dir/tmp/lulo-build" "$chroot_dir/etc/resolv.conf.build"
du -sx --block-size=1 "$chroot_dir" | cut -f1 > "$image/casper/filesystem.size"
mksquashfs "$chroot_dir" "$image/casper/filesystem.squashfs" \
    -noappend -comp xz -Xbcj x86 -b 1M -wildcards \
    -e 'proc/*' 'sys/*' 'dev/*' 'run/*' 'tmp/*' 'var/tmp/*' 'var/cache/apt/archives/*.deb'
endlog

# --- 5. Boot loaders ----------------------------------------------------------
log "boot loaders"
printf 'Lulo OS %s "Live" - amd64 (%s)\n' "$version" "$(date -u +%Y%m%d)" > "$image/.disk/info"
touch "$image/.disk/base_installable"
printf 'full_cd/single\n' > "$image/.disk/cd_type"
mkdir -p "$image/boot/grub/themes"
# The image's copy, which has the fonts chroot-setup.sh made with grub-mkfont.
cp -a "$chroot_dir/boot/grub/themes/lulo" "$image/boot/grub/themes/lulo"
sed "s/@VERSION@/$version/g" "$here/grub/grub.cfg" > "$image/boot/grub/grub.cfg"
# The signed CD GRUB reads $prefix/x86_64-efi/grub.cfg when it exists; keep
# one menu for both firmware types.
mkdir -p "$image/boot/grub/x86_64-efi"
printf 'source /boot/grub/grub.cfg\n' > "$image/boot/grub/x86_64-efi/grub.cfg"

shim=$(ls "$chroot_dir"/usr/lib/shim/shimx64.efi.signed.latest 2>/dev/null \
    || ls "$chroot_dir"/usr/lib/shim/shimx64.efi.signed)
mm=$(ls "$chroot_dir"/usr/lib/shim/mmx64.efi.signed 2>/dev/null \
    || ls "$chroot_dir"/usr/lib/shim/mmx64.efi)
cp "$shim" "$image/EFI/boot/bootx64.efi"
cp "$mm" "$image/EFI/boot/mmx64.efi"
cp "$chroot_dir/usr/lib/grub/x86_64-efi-signed/gcdx64.efi.signed" "$image/EFI/boot/grubx64.efi"

# EFI system partition image for El Torito and the USB GPT.
efi_img=$work/efiboot.img
rm -f "$efi_img"
mkfs.msdos -C -n LULO_EFI "$efi_img" 8192 >/dev/null
mmd -i "$efi_img" ::/EFI ::/EFI/boot
mcopy -i "$efi_img" "$image/EFI/boot/bootx64.efi" "$image/EFI/boot/mmx64.efi" \
    "$image/EFI/boot/grubx64.efi" ::/EFI/boot/

# Legacy BIOS: El Torito core image plus the hybrid MBR.
cat > "$work/early.cfg" <<'EOF'
search --no-floppy --file --set=root /.disk/info
set prefix=($root)/boot/grub
EOF
mkdir -p "$image/boot/grub/i386-pc"
cp "$chroot_dir"/usr/lib/grub/i386-pc/*.mod "$chroot_dir"/usr/lib/grub/i386-pc/*.lst \
    "$image/boot/grub/i386-pc/"
chroot_grub_mkimage() {
    # The chroot is unmounted here; grub-mkimage needs no /proc or /dev.
    chroot "$chroot_dir" grub-mkimage "$@"
}
cp "$work/early.cfg" "$chroot_dir/tmp/early.cfg"
chroot_grub_mkimage -O i386-pc-eltorito -p /boot/grub -c /tmp/early.cfg \
    -o /tmp/eltorito.img \
    biosdisk iso9660 part_msdos part_gpt fat ext2 search search_fs_file \
    normal configfile linux echo test
mv "$chroot_dir/tmp/eltorito.img" "$image/boot/grub/i386-pc/eltorito.img"
rm -f "$chroot_dir/tmp/early.cfg"
cp "$chroot_dir/usr/lib/grub/i386-pc/boot_hybrid.img" "$work/boot_hybrid.img"
endlog

# --- 6. ISO -------------------------------------------------------------------
log "xorriso"
rm -f "$out/$iso_name"
xorriso -as mkisofs \
    -r -J -joliet-long -l -iso-level 3 \
    -V "LULO_OS" \
    -o "$out/$iso_name" \
    --grub2-mbr "$work/boot_hybrid.img" \
    -partition_offset 16 \
    --mbr-force-bootable \
    -append_partition 2 0xEF "$efi_img" \
    -appended_part_as_gpt \
    -c /boot.catalog \
    -b /boot/grub/i386-pc/eltorito.img \
    -no-emul-boot -boot-load-size 4 -boot-info-table --grub2-boot-info \
    -eltorito-alt-boot \
    -e '--interval:appended_partition_2:all::' \
    -no-emul-boot \
    "$image"
(cd "$out" && sha256sum "$iso_name" > "$iso_name.sha256")
ls -l "$out"
endlog
