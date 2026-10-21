#!/bin/bash
# Runs inside the image chroot (build-iso.sh step 3). Installs the packages,
# the Lulo release, the branding, and prepares the live system (ADR 0031).
set -euo pipefail
build=/tmp/lulo-build

# DNS for the build. /etc/resolv.conf turns into a symlink to systemd-resolved's
# stub once that package installs, so give the link a target as well.
mkdir -p /run/systemd/resolve
cp "/etc/resolv.conf.build" /run/systemd/resolve/stub-resolv.conf
rm -f /etc/resolv.conf
cp "/etc/resolv.conf.build" /etc/resolv.conf

# Never start services in the chroot.
printf '#!/bin/sh\nexit 101\n' > /usr/sbin/policy-rc.d
chmod 0755 /usr/sbin/policy-rc.d

apt_install() {
    apt-get install --yes --no-install-recommends \
        -o Dpkg::Options::=--force-confold "$@"
}

apt-get update
apt_install apt-utils
# debootstrap takes the release pocket; bring it up to the current updates.
apt-get full-upgrade --yes -o Dpkg::Options::=--force-confold

packages=()
while read -r line; do
    line=${line%%#*}
    line=$(echo "$line" | xargs)
    [ -n "$line" ] || continue
    if [ "${line#\?}" != "$line" ]; then
        name=${line#\?}
        if apt-cache show "$name" >/dev/null 2>&1; then
            packages+=("$name")
        else
            echo "chroot-setup: optional package $name is not in the archive; skipped" >&2
        fi
    else
        packages+=("$line")
    fi
done < "$build/packages.txt"
apt_install "${packages[@]}"

# Signed boot files for the ISO and the installer (they coexist with grub-pc).
apt_install shim-signed grub-efi-amd64-signed

# The UEFI GRUB package the installer swaps in for grub-pc on a UEFI PC
# (calamares lulo_bootloader_packages), kept as .debs so installs work offline.
mkdir -p /usr/share/lulo-installer/debs
(
    cd /usr/share/lulo-installer/debs
    apt-get download grub-efi-amd64
)

# The Lulo release: apt resolves its dependencies from the archive.
apt_install "$build"/debs/*.deb

# Locale and time defaults (Calamares sets the user's own at install time).
sed -i 's/^# *en_US.UTF-8 UTF-8/en_US.UTF-8 UTF-8/' /etc/locale.gen
locale-gen
printf 'LANG=en_US.UTF-8\n' > /etc/default/locale
ln -sf /usr/share/zoneinfo/Etc/UTC /etc/localtime
echo "Etc/UTC" > /etc/timezone

# Branding: Plymouth splash and GRUB theme.
theme=/usr/share/plymouth/themes/lulo
mkdir -p "$theme"
cp "$build"/plymouth/lulo.plymouth "$build"/plymouth/lulo.script "$theme/"
cp "$build"/branding/plymouth/*.png "$theme/"
update-alternatives --install /usr/share/plymouth/themes/default.plymouth \
    default.plymouth "$theme/lulo.plymouth" 200
update-alternatives --set default.plymouth "$theme/lulo.plymouth"
mkdir -p /boot/grub/themes
rm -rf /boot/grub/themes/lulo
cp -a "$build/branding/grub-theme" /boot/grub/themes/lulo
mkdir -p /usr/share/icons/hicolor/scalable/apps
cp "$build/branding/lulo-installer.svg" /usr/share/icons/hicolor/scalable/apps/lulo-installer.svg
gtk-update-icon-cache -q /usr/share/icons/hicolor 2>/dev/null || true
mkdir -p /etc/calamares/branding/lulo
cp "$build"/branding/calamares/* /etc/calamares/branding/lulo/

# Font for the GRUB theme, from the Inter the session already ships.
inter=$(find /usr/share/fonts -iname 'Inter-Regular.*' | head -n 1)
if [ -z "$inter" ]; then
    inter=$(fc-match -f '%{file}' 'Inter:style=Regular' 2>/dev/null || true)
fi
if [ -z "$inter" ] || [ ! -f "$inter" ]; then
    inter=/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf
fi
echo "chroot-setup: GRUB theme font from $inter"
for size in 16 24; do
    grub-mkfont -s "$size" -n "Lulo Regular $size" -o "/boot/grub/themes/lulo/lulo-$size.pf2" "$inter"
done

# Live-session and installer helpers from the overlay.
chmod 0755 /usr/libexec/lulo-installer/* /usr/libexec/lulo-live/* /usr/libexec/lulo-ci-probe
chmod 0600 /etc/netplan/01-network-manager-all.yaml
systemctl enable lulo-live-session.service lulo-ci-probe.service
systemctl set-default graphical.target

# Lulo is the only session on the login screen. gdm3 pulls in GNOME's session
# files; divert them (dpkg keeps them out of the way across upgrades) so GDM,
# casper's autologin and new users all land in Lulo. Lulo's own safe mode is
# the recovery session.
mkdir -p /usr/share/lulo-os/diverted-sessions
for session in /usr/share/wayland-sessions/*.desktop /usr/share/xsessions/*.desktop; do
    [ -e "$session" ] || continue
    [ "$(basename "$session")" = rmac.desktop ] && continue
    dpkg-divert --local --rename --add \
        --divert "/usr/share/lulo-os/diverted-sessions/$(basename "$(dirname "$session")")-$(basename "$session")" \
        "$session"
done
ls /usr/share/wayland-sessions /usr/share/xsessions 2>/dev/null || true
test -f /usr/share/wayland-sessions/rmac.desktop

# GDM: Wayland only, and Lulo as the default session for every user.
mkdir -p /etc/gdm3
if [ -f /etc/gdm3/custom.conf ] && ! grep -q '^DefaultSession=' /etc/gdm3/custom.conf; then
    sed -i 's/^\[daemon\]$/[daemon]\nDefaultSession=rmac.desktop/' /etc/gdm3/custom.conf
fi
grep -n 'DefaultSession' /etc/gdm3/custom.conf || true

# The live kernel's initramfs: casper, Plymouth with the Lulo theme.
update-initramfs -u -k all

# Tidy up: no build-only state, a fresh machine id per install.
rm -f /usr/sbin/policy-rc.d
apt-get clean
rm -rf /var/lib/apt/lists/* /var/cache/apt/*.bin /var/log/*.log /var/log/apt/*
: > /etc/machine-id
rm -f /var/lib/dbus/machine-id
rm -f /etc/resolv.conf
ln -s ../run/systemd/resolve/stub-resolv.conf /etc/resolv.conf
rm -f /etc/ssh/ssh_host_* 2>/dev/null || true
