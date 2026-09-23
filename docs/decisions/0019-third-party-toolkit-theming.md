# ADR 0019 — Third-party apps get an original rmac GTK theme, and rmac hands its appearance to every toolkit

- **Status:** accepted 2026-09-23.
- **Scope:** `packaging/rmac-session/themes/rmac` (installed at `/usr/share/themes/rmac`),
  `packaging/rmac-session/gsettings/91_rmac-desktop.gschema.override`,
  `crates/rmac-gtk-settings/src/toolkit.rs`, the session supervisor's `monitor` loop, the
  Appearance pane in System Settings, `scripts/linux/start-rmac-session.sh` (Qt), the header-bar
  window rule in `packaging/rmac-session/shell.kdl`, and `design-lab/gtk-theme.html`.

## The question

Every app rmac did not write still looks like GNOME. Files, Text Editor, Settings, Firefox,
Chrome and LibreOffice put their window buttons on the right. They draw Adwaita's 34 px controls,
use GNOME's blue and GNOME's scroll bars, and use whatever font GNOME left behind. How much of the
Mac look can rmac give them without patching them? And which part of rmac owns it?

## What was measured

- **Controls and window chrome**, from `design-lab/chrome.html` and `menus.html` (macOS 26.2,
  dark):
  - Traffic lights are 14 pt circles on a 23 pt pitch. The first is centred 26 pt in on a
    toolbar window and 16 pt in on a title-bar window.
  - Toolbars are 52 pt tall and title bars 32 pt.
  - Push buttons and text fields are 24 pt tall with a 6 pt radius, and the focus ring is 3 pt.
  - The switch track is 36 × 16, with a 21 × 13 thumb.
  - Check boxes and radio buttons are 14 pt.
  - The slider track is 6 pt thick, with a 20 × 16 knob.
  - Menus have a 12 pt radius, 24 pt rows inset 5 pt with a 7 pt radius, and 1 pt separators
    inset 16 pt.
- **The overlay scroller**, measured today in TextEdit: after Page Down, the resting thumb is
  6 pt wide, fully round, and 3 pt in from the window edge. It is `#9A9A9A` over `#1E1E1E`,
  which is white at 55 %. The hover-widened scroller and the light-scheme thumb were not
  captured.
- **The accent** is `#1372F9`, rmac-design's `system_blue`.
- **The font** is Inter at 13 px. In GTK's 96 dpi points that is `Inter 9.75`.

## Decision

1. **An original GTK theme called `rmac`.** It contains only CSS, is MIT licensed, and uses no
   Apple artwork or code from other themes. It imports GTK's own built-in stylesheet (Adwaita
   for GTK 3, Default for GTK 4) and then restyles the controls a person meets most. Those are
   header bars (unified 52 pt, capsule buttons), the plain 32 pt title bar, traffic lights,
   push buttons, text fields and their focus ring, switches, check boxes and radio buttons,
   sliders, progress bars, overlay scrollers, list and sidebar selection, menus, popovers and
   tooltips. Both toolkits share the same two palette files.
   - GTK 4 (4.16 or later) switches palettes itself with `@media (prefers-color-scheme)`.
   - GTK 3 cannot follow the colour scheme. It loads `gtk-dark.css` when
     `gtk-application-prefer-dark-theme` is set, so rmac writes that key.
2. **Session defaults scoped to the rmac desktop.** A GSettings override with `:rmac` groups
   sets these, and only while `XDG_CURRENT_DESKTOP` names rmac, so the GNOME recovery session
   keeps its own look:
   - `gtk-theme 'rmac'`, `font-name 'Inter 9.75'`, `cursor-theme 'rmac'` and `cursor-size 24`;
   - `color-scheme 'prefer-dark'`, `accent-color 'blue'` and `overlay-scrolling true`;
   - `button-layout 'close,minimize,maximize:'`.

   The package verifier refuses an override that leaves the rmac desktop.
3. **rmac-gtk-settings owns everything that follows the Appearance choice.**
   `sync_toolkit_appearance` makes these changes:
   - It writes `color-scheme` and the nearest GNOME `accent-color` (Graphite maps to `slate`),
     which the Settings portal gives to libadwaita, GTK 4, Firefox, Chromium and Qt. It never
     writes `color-scheme` back when the preference is Automatic.
   - It merges rmac's keys into `~/.config/gtk-3.0/settings.ini` and `gtk-4.0/settings.ini`.
     Those keys are the theme, font, cursor, decoration layout,
     `gtk-primary-button-warps-slider=false` (a click in the track pages, as on the Mac), GTK 3's
     prefer-dark key and GTK 4's overlay scrolling. Every other key is kept.
   - It keeps managed stubs in `~/.config/gtk-{3,4}.0/gtk.css` that carry the accent. The GTK 4
     stub also loads `/usr/share/themes/rmac/libadwaita.css`.

   A file the user replaced (its marker line removed) or linked from elsewhere is left alone.
   System Settings calls the function right after saving a change. The session supervisor polls
   the theme store every 5 s. That covers login, hand edits and a host scheme change while the
   preference is Automatic.
4. **Qt uses Qt's GTK 3 platform theme.** `QT_QPA_PLATFORMTHEME=gtk3` gives Qt apps the rmac
   palette, Inter and dark mode. rmac-session recommends `qt6-gtk-platformtheme`.
   `QT_WAYLAND_DECORATION=adwaita` is set only when the QAdwaitaDecorations plugin is installed.
5. **Header-bar apps take the toolbar corner.** niri rounds `org.gnome.*` apps, Firefox and
   Chrome at 27 pt, like Mac toolbar windows. Everything else keeps 16 pt. Corners and shadows
   always come from niri, not from GTK.

## What this cannot reach (known limits)

- **libadwaita** ignores `gtk-theme`. rmac reaches it only through the user stylesheet: its
  colour variables, the accent, window controls, header-bar height and the standard widgets.
  AdwHeaderBar still centres its title. The layout and spacing of AdwToolbarView and AdwDialog,
  adaptive breakpoints, and animations are libadwaita's.
- **GTK layout.** Titles stay centred (the Mac left-aligns them after the lights). The switch
  thumb is 20 × 14 with 1 pt insets, because GTK gives the thumb half the track in whole pixels.
  Window-control glyphs are the icon theme's symbolic images. Menus and popovers are opaque,
  because GTK cannot blur what is behind a popup.
- **GTK 3 dark mode** follows rmac's own choice through `settings.ini`. A GTK 3 app already
  running switches when GTK rereads its settings, which in practice means on its next launch.
- **Qt** keeps the Fusion widget style, now in rmac colours. The GTK 3 platform theme opens GTK's
  own file dialog, not rmac's portal Open panel. Choosing `xdgdesktopportal` would get the
  portal dialog but lose the palette and font, so appearance wins until Qt can have both.
- **Firefox and Chromium** follow the colour scheme, the font and the left-hand button layout
  through the portal, and Chromium follows the GTK theme when set to "GTK". Their tab strips are
  their own, and Chrome's "Classic" theme ignores GTK completely.
- **Electron** apps behave like Chromium when they run on Wayland with window decorations.
  Apps that force X11 or draw a custom title bar are out of reach.
- **LibreOffice** uses the GTK 3 VCL plugin and so gets the theme, the font and dark mode. Its
  toolbars and icon set stay its own.
- **Light appearance** control fills and the hover-widened scroller are starting values (S)
  until the Mac is captured in Light.

## Verification

- `design-lab/gtk-theme.html` sets each GTK widget next to the measured Mac control in dark and
  light.
- `python3 scripts/test_session_package.py` stages the theme and the override, and proves the
  verifier refuses a widened override.
- The unit tests in `crates/rmac-gtk-settings` cover the `settings.ini` merge (idempotent,
  keeps foreign keys), the accent mapping for every Appearance swatch, Automatic never writing
  the scheme, and a user-owned `gtk.css` being left alone.
- Still to be checked on the reference PC: GTK Widget Factory 3 and 4, Files, Text Editor,
  Settings, Firefox, Chrome, LibreOffice and a Qt app, in both appearances.
