# Launcher and Spotlight domain

`rmac-launcher` is the framework-neutral contract for D7/D8. It is shared by
the future centered launcher overlay and provider adapters; providers supply
typed candidates but cannot decide privacy admission, cross-category order,
selection, or activation fallback.

## Provider privacy

Every provider declares whether it needs private content and/or network access.
The versioned shell-settings policy is checked before a provider receives a
query. Missing policy uses the privacy-first defaults: local public providers
such as applications/settings/calculator run, while file-name/content and
network providers require explicit permission. Disabled providers never enter
the request.

Duplicate provider IDs are admitted once. A returned result must use its
requested provider ID, declared category, and a nonempty local ID. A batch that
spoofs another provider/category is rejected and exposed as a provider error,
so it cannot bypass privacy policy or distort ranking.

## Query lifecycle and cancellation

Beginning a query cancels the previous `Cancellation` token, increments a
generation, clears old batches, and records the exact admitted providers. File
adapters can pass the token's atomic flag directly to `rmac-search::Options`.
Results from an older generation, an unrequested provider, or a cancelled
request are ignored.

Providers return one bounded batch or a typed error. One failure does not erase
other categories. Pending/error state remains visible for loading and honest
partial-result UI. Escape closes the overlay and cancels every outstanding
provider; there is no background indexing loop in this domain.

## Ranking and keyboard behavior

Ranking normalizes whitespace/case and scores exact, prefix, word-prefix,
substring, then subsequence matches. Title matches outrank subtitle matches.
Small category and provider-normalized recency weights break otherwise useful
ties; result ID/title supply deterministic final ordering. Recency is a bounded
0–100 signal, never raw wall time.

A total result limit and per-category cap prevent one broad provider from
crowding out applications, settings, calculator, or files. Selection retains
its stable result ID as later providers complete, falls back to the first
result only when needed, and wraps in both directions.

Primary activation is exact. Alternate activation (for example Reveal for a
file) returns only a declared alternate and never silently falls back to the
primary action. Actions carry parsed shell-free application launch specs,
setting pane IDs, private file paths, or calculator text; default logs must not
print private action payloads.

## Local providers

`rmac-launcher-providers` implements four bounded, background-safe adapters:

- Applications retain the exact parsed desktop-entry launch specification; no
  result reconstructs a command from display text.
- Settings match titles, subtitles, and synonyms but return stable pane IDs,
  with duplicate and empty IDs excluded.
- Files use recent documents for an empty query and filename search otherwise.
  They accept only an absolute root, return only absolute deduplicated paths,
  pass the query cancellation flag into `rmac-search`, and declare private
  content before the request is admitted. Open is primary and Reveal is the
  explicit alternate action.
- Calculator evaluates finite arithmetic with precedence, parentheses, unary
  signs, a 256-byte input bound, and no scripting or function surface. Its
  result is a typed copy-text action.

Before an adapter runs, its complete descriptor must exactly match one admitted
to the privacy-filtered request. Cancellation before or after provider work
suppresses the batch; cancellation observed during work becomes an explicit
provider error. Providers never execute their returned action.

## Activation

`rmac-launcher-system` executes the selected typed action once. It validates
nonempty application and pane IDs, absolute file paths, and nonempty copied
text before calling a backend. Application launch remains off the UI executor
and consumes the exact parsed launch specification. Open and Reveal use
different portal-backed operations, preserving the alternate-action contract.

Clipboard writes and Settings navigation are surface operations: the future
GPUI overlay supplies them from its live application context instead of a
detached subprocess. Successful receipts expose only the activation ID and
outcome kind. Default error formatting does not contain a file path, copied
text, pane ID, or backend detail; UI code may deliberately inspect typed kind
and detail to produce a suitable private on-screen error.

The immediate-focus GPUI overlay, global shortcut journey, Orca semantics,
privacy Settings pane, and performance evidence remain pending. This slice does
not mark D7/D8 complete.
