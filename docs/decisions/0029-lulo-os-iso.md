# ADR 0029 — Lulo OS bootable USB image: debootstrap + casper, Calamares installer

- Status: accepted
- Date: 2026-10-20
- Follows: ADR 0007 (compositor choice), ADR 0027 (hardware profile)
- Branch: `op/lulo-os-iso`

## Context

Until now Lulo OS installed as packages on top of an existing Ubuntu 26.04
(`install.sh`, docs/install.md). The owner wants anyone with a Windows PC,
especially a low-spec one, to be able to try Lulo OS from a USB stick and then
install it next to Windows or in place of it. The constraints:

- no paid services, and builds only in GitHub Actions (the Mac has 13 GB free
  and the Ubuntu laptop is shared);
- the image has to fit under GitHub's 2 GiB release-asset limit;
- it has to boot with Secure Boot on, on UEFI and on legacy BIOS PCs;
- "install alongside Windows" has to shrink the Windows partition safely;
- it should stay light on 4 GB machines and keep the move to Lulo's own OS base
  (memory: long-term own OS) open.

## Options considered

| Option | For | Against |
|---|---|---|
| **Remaster the official Ubuntu desktop ISO** (Cubic style) | Ubuntu's own boot chain and installer | The 26.04 desktop ISO is about 6 GB, with layered squashfs images and a snap-based installer. Taking GNOME and the snaps out of the layers is fragile, and the result is still far over 2 GB. |
| **Ubuntu `live-build`** | Scripted | Ubuntu's fork is unmaintained (livecd-rootfs is what Canonical actually uses, and it is tied to Launchpad). |
| **mkosi** | Modern and declarative, makes disk images well | It has no casper live boot, no hybrid ISO with El Torito BIOS and shim, and no installer. We would write all of that ourselves. |
| **debootstrap + casper + squashfs + xorriso** (chosen) | The same pieces the Ubuntu flavours use: the archive's signed shim and GRUB, casper for the live session, one xz squashfs. Every package comes from the Ubuntu archive or the Lulo release. About 300 lines of shell, all in this repo. | We maintain the build script. |

| Installer | For | Against |
|---|---|---|
| Ubuntu's Flutter installer (subiquity) | Ubuntu's own, with autoinstall | Ships as a snap (snapd on a low-spec PC, and seeding snaps into a custom image), and it expects Ubuntu's layered squashfs catalogue. |
| **Calamares 3.3.14** (chosen) | In the 26.04 archive and used by Lubuntu 26.04 with casper, so the Ubuntu patches for GRUB/shim, debconf and LUKS are already in it. Its partition step offers Erase disk, Install alongside (KPMcore + `ntfsresize`, which refuses a hibernated or dirty NTFS volume) and Replace a partition. Qt, but it is not part of the daily desktop and it is removed from the installed system. | Not GPUI, so it does not look exactly like the rest of Lulo. Branding (colours, logo, slides) closes most of the gap. |

## Decision

### Image

- **Base:** Ubuntu 26.04 (`resolute`) from `archive.ubuntu.com`, built with
  `debootstrap --variant=minbase` in a GitHub `ubuntu-26.04` runner, plus
  `ubuntu-minimal`, `linux-generic` (signed kernel, `linux-firmware`), the
  Intel and AMD microcode, Mesa (GL, Vulkan lavapipe/radv/anv, VA-API), NetworkManager,
  BlueZ, PipeWire, GDM, Plymouth and `ubuntu-drivers-common`. No snaps, no
  GNOME session, no LibreOffice. `--no-install-recommends` everywhere, and
  the recommends we want (keyd, pkexec, the Qt GTK platform theme,
  onnxruntime, evolution-ews-core) are listed by name.
- **Lulo:** `rmac-apps`, `rmac-session`, `niri` and `xwayland-satellite` are
  taken from the GitHub Release named by the build. They are checked
  against that release's `SHA256SUMS` and its build-provenance attestation.
  The hardware profile (ADR 0027) is part of those binaries, so the live
  session adapts to the PC with no extra step.
- **Live boot:** casper, with one `filesystem.squashfs` (xz, BCJ x86 filter,
  1 MiB blocks). The live user is `lulo` (passwordless, autologin through
  GDM). `lulo-live-session.service` runs only when the kernel command line
  has `boot=casper`. It sets the live user's session to `rmac`, adds
  **Install Lulo OS** to the Dock and marks the setup assistant done, so the
  live desktop opens straight into Lulo with the installer in reach.
- **Boot chain:** the ISO is a hybrid image. One file works burned to a DVD,
  written to USB by Rufus (DD mode) or balenaEtcher, or copied file by file
  by Rufus (ISO mode).
  - UEFI: `EFI/boot/bootx64.efi` is Ubuntu's Microsoft-signed shim, and
    `grubx64.efi` is Canonical's signed `gcdx64.efi`. Both are copied from
    `shim-signed` and `grub-efi-amd64-signed`, so the image boots with
    Secure Boot on. They live on an appended EFI system partition and also
    in the ISO 9660 tree.
  - Legacy BIOS: an El Torito `i386-pc-eltorito` GRUB image, with GRUB's
    `boot_hybrid.img` as the MBR.
- **Branding:** the GRUB theme and the Plymouth script theme are drawn from
  the original Lulo artwork in `assets/brand` and
  `packaging/rmac-session/wallpapers` (docs/brand.md). There are no Apple
  assets. The installed system keeps the GRUB theme in `/boot/grub/themes/lulo`,
  so it is readable with or without disk encryption.

### Installer

Calamares runs as root from the live user's session (`sudo`, which casper
leaves passwordless). It has a Lulo branding component and this sequence:

`welcome → location → keyboard → partition → users → summary`, then
`partition, mount, unpackfs, machineid, fstab, locale, keyboard, localecfg,
luksbootkeyfile, shellprocess@lulo_cleanup, users, displaymanager, networkcfg,
hwclock, contextualprocess@lulo_bootloader_packages, initramfscfg, initramfs,
grubcfg, bootloader, packages, umount`.

- `lulo_cleanup` deletes the live-only files (the installer's Dock entry and
  launcher, the live session service, the casper configuration) from the
  target before the first user is created. `packages` then removes
  `calamares` and `casper`.
- `lulo_bootloader_packages` replaces `grub-pc` with `grub-efi-amd64`
  (shipped as `.deb` files in the image, so it works offline) on a UEFI PC.
  It leaves a BIOS PC alone. GRUB's `grub-install` then puts the signed shim
  and GRUB on the EFI system partition. The EFI loader id stays `ubuntu`,
  because Canonical's signed GRUB looks for its configuration there.
  `installEFIFallback` also writes `\EFI\BOOT\BOOTX64.EFI` for firmware that
  forgets boot entries.
- `os-prober` is installed and `GRUB_DISABLE_OS_PROBER=false`, so the
  installed GRUB menu lists Windows Boot Manager on a dual-boot PC.
- Partitioning: Erase disk, Install alongside, Replace a partition, or Manual,
  all from Calamares. ext4 with a swap file by default, and optional LUKS2
  encryption with a separate unencrypted `/boot`, as Lubuntu 26.04 does.

### Build and test

`.github/workflows/lulo-os-iso.yml` runs on manual dispatch and after every
successful tagged `Release` run (`workflow_run`). It builds
`lulo-os-<version>-amd64.iso` with a `.sha256` file and fails if the image
reaches 2 GiB. It then boots the image under QEMU/KVM:

- the live session, with OVMF and Secure Boot enforced (Microsoft keys
  enrolled);
- the live session with SeaBIOS;
- a full Calamares install to a 24 GB virtual disk, driven through the real
  UI by screenshots and OCR (`scripts/iso/qemu_iso_test.py`), then a boot
  of the installed disk to GDM, a login, and the Lulo session.

`lulo-ci-probe.service` reports progress on the VM's serial port. It runs
only when QEMU passes the `opt/org.lulo.ci` fw_cfg entry, so on a real PC its
only cost is one skipped unit. Screenshots and serial logs are CI artifacts
only. On a tag, the tested ISO and its checksum are attached to the release.

## Consequences

- The image depends on the Ubuntu archive at build time and on the matching
  GitHub Release. A release ISO is reproducible in content but not
  bit-for-bit (debootstrap pulls the archive's state on that day).
- Proprietary NVIDIA drivers are not on the image (size, and the open
  driver works out of the box). `ubuntu-drivers-common` is installed, so the
  installed system can add them the Ubuntu way.
- Branding and live-session files are copied into the image, not packaged,
  so a later release changes them only for new installs. Packaging them as
  `lulo-os-branding` is follow-up work.
- Calamares, GDM and casper are Ubuntu-only choices for now. When Lulo moves
  to its own base, the image script is the one place to change.
