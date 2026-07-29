# Application metadata localization

`rmac-apps.pot` is the authoritative message template for application names,
generic names, summaries, and desktop actions. `LINGUAS` lists reviewed
translations and every listed locale has a complete PO catalog. Installed
desktop and AppStream metadata contains the reviewed translations directly, so
launchers and software centers do not depend on an rmac process to localize it.

When a catalog changes, merge its translations into both the matching
localized desktop keys and the matching `xml:lang` AppStream elements. The
package fixture rejects missing, stale, fuzzy, or untranslated catalog entries.
Runtime application UI localization is a separate product-wide gate.
