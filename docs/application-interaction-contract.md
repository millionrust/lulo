# Application interaction contract

This contract keeps rmac applications feeling like one desktop rather than a
collection of unrelated windows. It applies to every first-party application
and shell-owned application panel.

## Shared ownership

`rmac-ui` owns the first-party `Dialog`, `Alert`, `ContextMenu`, button roles,
focus presentation, semantic theme tokens, and application shortcut
vocabulary. Applications compose those primitives with domain-specific text
and actions; they do not create visually independent replacements for the same
interaction.

Dialogs have one clear purpose. A safe primary action may be the default.
Destructive actions use the destructive role, are never initial focus, and are
not activated by an unqualified Enter unless the user is already inside the
matching reviewed confirmation. Escape closes only the topmost transient
surface and does not accept or perform destructive work.

Context menus preserve action order, use separators to expose meaningful
groups, and use the shared danger presentation for irreversible actions. An
application may omit an unavailable action. It must not display a shortcut hint
for a command that is not registered in that application context.

## Shortcut source of truth

Every reusable application command is represented by one
`rmac_ui::shortcuts::Shortcut`. The value pairs:

- GPUI's portable keystroke used by `KeyBinding`; and
- the exact compact hint rendered in a first-party menu.

The constructor and arbitrary shortcut-hint menu path are private to `rmac-ui`.
Consumers register a named semantic constant and pass that same constant to
`ContextMenu::command_item` or `ContextMenu::danger_command_item`. This makes a
binding and its visible hint one change rather than two independent strings.

Portable bindings use `cmd`, which maps to the platform command modifier.
Multi-modifier GPUI strings place it first, as in `cmd-option-key` or
`cmd-shift-key`. Visible hints follow the familiar symbol order, such as
`⌥⇧⌘`.
Bare navigation keys use the shared Enter, Escape, Space, and arrow constants.
Platform-specific commands may be registered conditionally, but their semantic
shortcut remains shared.

The vocabulary test rejects empty values and legacy `shift-cmd` or
`option-cmd` ordering, and requires duplicate keystrokes to have identical
hints.

## Current adoption

Text Editor, Files, Terminal, System Monitor, App Drawer, and System Settings
register their application-level bindings from the shared vocabulary. Files
and Terminal context menus consume the same shortcut values as their bindings;
App Drawer does the same for Open. Menu entries without a registered direct
binding, including Open With, Restore, Profiles, and Show in Folder, do not
invent hints.

Notes remains the named adoption gate while its document producer and current
interaction work are in progress. Shell surfaces that interpret raw compositor
key events may share the bare-key vocabulary where practical, but they do not
pretend those event handlers are GPUI application bindings.

## Linux acceptance

The contract is not release-proven until the Ubuntu/niri journey records:

1. every documented application shortcut;
2. keyboard-only menu and dialog traversal;
3. accurate visible hints at supported scales;
4. topmost-first Escape behavior;
5. safe Enter/default-button behavior;
6. destructive confirmation and cancellation;
7. focus return after menus and dialogs close; and
8. Orca names, roles, state, actions, and announcements after the framework
   accessibility gate passes.
