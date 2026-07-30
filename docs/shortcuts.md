# Keyboard shortcuts

The GlobalShortcuts portal owns consent and may let the user choose different
bindings. System Settings shows the active backend and returned trigger text.
The table below lists the stable niri fallback defaults, not an unconditional
claim about the user's current portal bindings.

| Action | Default |
| --- | --- |
| Open launcher | `Super`–`Space` |
| Open Applications | `Super`–`A` |
| Open Notification Center | `Super`–`N` |
| Open Quick Settings | `Super`–`Control`–`C` |
| Lock the session | `Super`–`Control`–`Q` |

Only the lock shortcut is allowed while locked. The fallback invokes a fixed,
allow-listed dispatcher with separate arguments; it never converts shortcut
text into a shell command. Enable the niri include only when the runtime status
reports `fallback-required`.

## Application conventions

First-party apps share one semantic shortcut vocabulary and show the same
binding in menus. GPUI's portable command modifier owns the actual binding,
while first-party menus intentionally retain the familiar command symbols.

Common commands include New, Open, Save, Save As, Close, Find, Select All,
Copy, Cut, Paste, Undo, Redo, Print, Preferences/Settings, and app-specific
actions where available. A menu does not display a shortcut hint unless that
command is registered in the current context.

Enter activates only a safe default action. Escape dismisses the topmost menu,
popover, or dialog and never confirms destructive work. Arrow keys move
through lists and menus; Tab and Shift–Tab follow the visible focus order.

## Changing global shortcuts

Open System Settings → Spotlight → Shortcut. When GlobalShortcuts v2 is
available, **Configure** opens the portal-owned configuration UI. With the niri
fallback, edit or disable the generated include through the documented niri
configuration path; never enable both backends.

The implementation and recovery contract is documented in
[Global shortcuts](global-shortcuts.md).
