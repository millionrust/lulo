# Install Lulo OS from a USB stick

You can run Lulo OS on a PC that has Windows on it today. Start it from a USB
stick to try it first. Nothing on the PC changes while you try it. If you like
it, install it next to Windows (you pick one each time the PC starts) or in
place of Windows.

## What you need

- A 64-bit PC from about 2010 or later. 4 GB of memory is enough, and 2 GB
  works for trying it.
- A USB stick of **4 GB or more**. Everything on the stick is erased.
- About **25 GB of free space** on the PC's disk if you want to install.
- About half an hour.

## 1. Download Lulo OS

1. Open the [Lulo OS releases page](https://github.com/millionrust/lulo/releases).
2. Under the newest release, download `lulo-os-<version>-amd64.iso`. It is a
   bit under 2 GB. If you like, also download the `.sha256` file next to it.

You can check that the download is complete (optional). In Windows, open
**PowerShell** from the Start menu and type:

```powershell
Get-FileHash $HOME\Downloads\lulo-os-*-amd64.iso
```

The long code it prints should match the code in the `.sha256` file. You can
open that file in Notepad.

## 2. Make the USB stick

Use one of these free tools.

### Rufus (Windows)

1. Download Rufus from [rufus.ie](https://rufus.ie) and open it. It does not
   need installing.
2. Under **Device**, choose your USB stick.
3. Click **SELECT** and choose the Lulo OS `.iso` file.
4. Leave **Partition scheme** as Rufus suggests: **GPT** for most PCs, or
   **MBR** for a PC older than about 2012.
5. Click **START**.
6. If Rufus asks how to write the image, choose **Write in DD Image mode**,
   then click **OK**. It copies the image exactly as it is, which is the most
   reliable way.
7. When the bar says **READY**, close Rufus.

### balenaEtcher (Windows, Mac or Linux)

1. Download balenaEtcher from [etcher.balena.io](https://etcher.balena.io)
   and open it.
2. Click **Flash from file** and choose the Lulo OS `.iso` file.
3. Click **Select target** and choose your USB stick.
4. Click **Flash!** and wait until it says the flash is complete.

## 3. Before you install next to Windows

You can skip this section if you only want to try Lulo OS.

- **Back up your files.** Installing next to Windows shrinks the Windows
  partition, and that is designed to be safe. But a power cut in the middle
  can still lose data, so keep a copy of anything you can't replace.
- **Plug in the charger** if it's a laptop.
- **Turn off Fast Startup.** Fast Startup leaves the Windows disk half asleep,
  and the installer will not shrink a disk in that state. Open **Control
  Panel → Hardware and Sound → Power Options → Choose what the power buttons
  do**. Click **Change settings that are currently unavailable**, untick
  **Turn on fast startup**, then click **Save changes**.
- **BitLocker or Device encryption.** Many newer PCs encrypt the Windows disk.
  The installer cannot shrink an encrypted disk, so turn encryption off first:
  open **Settings** and search for **Device encryption** or **Manage
  BitLocker**, then turn it off and wait until decryption finishes. It can
  take an hour or more. Also write down your BitLocker recovery key (search
  for "BitLocker recovery key" at [aka.ms/myrecoverykey](https://aka.ms/myrecoverykey)).
  Some PCs ask for it once after Lulo OS is installed.
- Then **shut Windows down fully**. Hold **Shift** while you click **Shut down**.

## 4. Start the PC from the USB stick

1. Plug in the USB stick and turn the PC on (or restart it).
2. Straight away, tap the **boot menu key** for your PC's brand again and
   again until a menu appears:

   | Brand | Boot menu key |
   |---|---|
   | Acer | F12 (turn on "F12 Boot Menu" in setup with F2 if nothing happens) |
   | ASUS | F8 on desktops, Esc on laptops |
   | Dell | F12 |
   | HP | Esc, then F9 |
   | Lenovo | F12, or the small Novo button or pinhole next to the power button |
   | MSI | F11 |
   | Samsung | F2 for setup, then the Boot tab, or Esc on some models |
   | Toshiba / Dynabook | F12 |
   | Huawei / Honor | F12 |
   | Gigabyte | F12 |
   | ASRock | F11 |
   | Microsoft Surface | Hold **Volume down** and press and release the power button |

   **Another way, from Windows:** open **Settings → System → Recovery**. Next
   to **Advanced startup**, click **Restart now**, then choose **Use a device**
   and pick the USB stick.

3. In the menu, choose the USB stick. It may show as "UEFI: USB", the stick's
   brand name, or "EFI USB Device".

**Secure Boot can stay on.** Lulo OS starts with the same Microsoft-signed
boot files as Ubuntu.

## 5. Try Lulo OS

A dark menu with the Lulo logo appears, and after a few seconds **Try or
Install Lulo OS** starts by itself. Lulo OS opens straight into its desktop.
You don't need a password.

- Everything runs from the USB stick and from memory. **Nothing on your disk
  changes**, and anything you save is gone when you restart.
- Connect to Wi-Fi from the menu bar to try the internet and the apps.
- If the screen stays black or flickers, restart and choose **Lulo OS (safe
  graphics)** in the menu.

To leave, choose **Shut Down** from the Lulo menu, then take out the USB
stick. Your PC starts Windows as before.

## 6. Install Lulo OS

Click **Install Lulo OS** in the Dock (the orange Lulo icon). The installer
walks you through a few pages:

1. **Welcome:** your language.
2. **Location:** your time zone.
3. **Keyboard:** your keyboard layout. Type in the test box to check it.
4. **Partitions:** where Lulo OS goes. Choose one:
   - **Install alongside**: keeps Windows. Drag the divider to choose how much
     space Lulo OS gets (25 GB or more is comfortable). The installer shrinks
     Windows to make room. Each time the PC starts, you choose Lulo OS or
     Windows.
   - **Erase disk**: removes Windows and everything on the disk, and uses the
     whole disk for Lulo OS.
   - **Replace a partition**: puts Lulo OS in place of one partition you pick,
     for example an old Linux install.
   - **Encrypt system** (a tick box): asks for a password every time Lulo OS
     starts. Choose it if the PC could be lost or stolen.

   If you only see **Erase disk** and expected **Install alongside**, Windows
   is probably still encrypted or Fast Startup is still on (see section 3).
   Click **Cancel**, fix that in Windows, and start again.
5. **Users:** your name, the name you sign in with, the computer's name and a
   password.
6. **Summary:** check it, then click **Install**, then **Install now**.

Installing takes 10 to 30 minutes. When it says **All done**, leave **Restart
now** ticked and click **Done**. Take out the USB stick when the screen asks
you to.

The first time Lulo OS starts, it walks you through a short setup, like a new
Mac does.

## Choosing Lulo OS or Windows when the PC starts

If you installed alongside Windows, the PC shows the Lulo menu for a few
seconds each time it starts:

- **Lulo OS** starts Lulo OS. It is the default.
- **Windows Boot Manager** starts Windows.

Use the arrow keys, then press **Enter**.

## Going back to Windows

### You installed alongside Windows and want only Windows again

1. Start **Windows**.
2. Right-click the Start button and choose **Disk Management**.
3. Find the Lulo OS partitions. They are the ones Windows can't read, usually
   right after your C: drive and without a drive letter. Right-click each one
   and choose **Delete Volume**. Don't touch the small **EFI System
   Partition**, the **Recovery** partition, or C:.
4. Right-click **C:** and choose **Extend Volume** to take the space back.
5. Remove the Lulo start-up menu. Open **Terminal (Admin)** (right-click the
   Start button) and type these two commands:

   ```
   mountvol S: /S
   rmdir /S /Q S:\EFI\ubuntu
   ```

   Restart. The PC starts Windows directly. (The firmware may name Lulo's
   boot entry "ubuntu", because Lulo OS uses Ubuntu's signed boot files.)

### You erased Windows and want it back

Use Microsoft's free **Media Creation Tool** on any Windows PC to make a
Windows USB stick, start the PC from it the same way (section 4), and install
Windows. If the PC came with Windows 10 or 11, Windows activates by itself
after it connects to the internet.

## If something goes wrong

- **The PC starts Windows instead of the USB stick.** Tap the boot menu key
  sooner, try another USB port (a port on the PC itself, not a hub), or use
  the "Advanced startup" way in section 4.
- **"Secure Boot violation" or "Verification failed".** Update the PC's
  firmware (BIOS) from the manufacturer's website, then try again. As a last
  resort you can turn Secure Boot off in the firmware setup.
- **The screen stays black.** Choose **Lulo OS (safe graphics)** in the menu.
- **Windows asks for a BitLocker recovery key** after installing. Enter the
  key you wrote down in section 3. It asks only once.
- More help: [Troubleshooting](troubleshooting.md) and the
  [issue tracker](https://github.com/millionrust/lulo/issues).
