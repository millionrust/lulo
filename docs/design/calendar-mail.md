# Calendar and Mail for Beta 1

Decision record: `docs/decisions/0022-calendar-mail-accounts-and-data.md` (GOA holds accounts,
EDS serves calendars, Mail has its own Rust engine). Mocks: `design-lab/calendar.html`,
`design-lab/mail.html`, `design-lab/internet-accounts.html`. Pending behaviour scenarios:
`docs/behavior-pending/{calendar,mail,settings}/`. Parity rows: APP-01, APP-13, SET-63.

Everything marked **(verify on Mac)** comes from knowledge of macOS 26, not a capture. A later
read-only Mac inventory (`tests/inventory/mac`) must confirm it before the port fixes numbers.
The mocks follow the Files/Settings measurements already in `design-lab/`; Calendar- and
Mail-specific sizes are estimates until captured.

## 1. Accounts (shared)

**Day one**
- System Settings ▸ **Internet Accounts** pane: account list (icon, name, address, services
  summary such as "Mail, Calendars"); selecting one shows its service switches (Mail,
  Calendars, Contacts), description, and **Delete Account…**.
- **Add Account…** sheet (also from Mail ▸ Add Account… and Calendar ▸ Add Account…): e-mail
  field plus provider list (Google, Microsoft, Yahoo, iCloud, Other Mail Account…, Other
  Calendar Account…). Typing an address and pressing Continue picks the provider by domain/MX.
- Google and Microsoft: "Continue in your browser" step with a spinner and Cancel; the browser
  returns to Lulo, the sheet shows the service switches (all on), **Done**.
- Yahoo and iCloud: name, address, app-specific password, a "Get an app-specific password"
  link button, Sign In. Other: name, address, password; servers auto-configured, manual IMAP/SMTP
  fields appear only if discovery fails.
- Errors are sentences ("Lulo couldn't sign in to Google. Check your connection and try again."),
  never codes.

**Mac behaviour to match (verify on Mac):** provider list order iCloud, Microsoft Exchange,
Google, Yahoo!, AOL, Other Account…; sign-in happens in a web sheet; after sign-in a checklist of
apps to use with the account; Delete asks "Are you sure…? This removes it from all apps".

**Deferred:** Exchange on-premises (EWS), AOL, CardDAV-only, LDAP, Kerberos, per-account
certificate trust, Lulo's own OAuth client registrations.

## 2. Calendar

**Day one**
- Window: floating glass sidebar (calendar list grouped by account, colour checkboxes, mini
  month at the bottom), toolbar with sidebar toggle, **+** (new event), Day/Week/Month/Year
  segmented control, `‹ Today ›`, inbox (invitations) and search.
- Views: **Day** (hour grid + event detail column), **Week** (all-day strip, hour grid,
  red now-line), **Month** (5–6 week grid, events as coloured dots/bars, "N more"), **Year**
  (12 mini months, busy days tinted). ⌘1–⌘4, ⌘T Today, ⌘←/⌘→ previous/next; trackpad scroll
  and kinetic scrolling move days/weeks/months as on the Mac.
- Create: double-click empty time, drag across time, **+**/⌘N ("New Event" at the next hour),
  all-day by double-clicking the all-day strip/month cell.
- Edit: event popover inspector (title, location, all-day, starts/ends, time zone read-only,
  repeat, alert, calendar, invitees, notes, URL). Drag to move, drag the bottom edge to resize,
  ⌘C/⌘V/⌘D duplicate, Delete with "Delete this event only / all future events" for repeats.
  Undo/redo for create/move/edit/delete.
- Calendars: show/hide (checkbox), colour, rename, New Calendar (local or on an account),
  File ▸ New Calendar Subscription… (webcal/https ICS), File ▸ Import… (.ics), Export.
- Invitations: events with attendees show organiser and attendee status; Accept / Maybe /
  Decline in the inspector and the inbox popover. Google and iCloud servers send the replies
  (CalDAV scheduling); for other servers the reply is sent by Mail as iMIP (MAIL-8).
- Reminders: alerts fire as Lulo notifications through `calendar-agent` even when Calendar is
  closed; Snooze and Close; clicking opens the event. Default alerts per account in Settings.
- Search: finds events by title, location, notes and people across visible calendars; results
  list under the toolbar, Return jumps to the event.
- Settings (⌘,): General (default calendar, start of week, day starts/ends, show N days,
  show week numbers, default alert times), Accounts (refresh interval, enable/disable),
  Alerts. **(verify on Mac: exact panes and labels)**
- Integration: menu-bar clock and the desktop Calendar widget open Calendar on that date; the
  widget shows the next events (CAL-8).

**Mac behaviour to match (verify on Mac):** Week view's today column header has the date in a
red circle; the now-line is red with a dot on today's column and the time in the gutter; weekend
columns tinted; all-day events as solid bars, timed events as tinted blocks with a 3 pt coloured
left edge; Month view day numbers top-right; Year view heat-tints busy days; inspector opens as a
popover pointing at the event; Calendar ▸ Settings ▸ General "Show Birthdays/Holidays calendar".

**Deferred:** natural-language Quick Event, travel time, attachments on events, time-zone
override UI, delegation, availability (free/busy) panel, Birthdays/Holidays calendars, Reminders
(VTODO) list, printing, Siri suggestions.

## 3. Mail

**Day one**
- Window: sidebar (Favourites: All Inboxes, Flagged, Drafts, Sent; then each account's
  mailboxes with special-use icons and unread counts), message list (unread dot, sender bold,
  date, subject, two-line preview, attachment/flag glyphs, thread counts), message viewer
  (header, avatar initials, To/Cc, body, attachments strip). Toolbar: filter by unread,
  compose, archive, trash, junk, reply / reply all / forward, flag, move, search.
- Read: opening marks read (Mac default; verify the delay), ⇧⌘U toggles, threads collapsed
  by conversation (View ▸ Organize by Conversation on by default), Return opens a message in
  its own window.
- Compose: separate window (⌘N), To/Cc/Bcc with address completion (recents + EDS contacts),
  Subject, From popup (per account aliases), signature popup, rich text by default with plain
  text option, attach (⇧⌘A, drag in), send ⇧⌘D, drafts autosave to the server Drafts mailbox,
  Outbox while offline. Reply ⌘R, Reply All ⇧⌘R, Forward ⇧⌘F with quoting as the Mac
  (`On <date>, <name> wrote:`; verify exact wording).
- Organise: delete (Trash), archive (Archive mailbox), move (drag or ⌃⌘M Move to…), flag
  (⇧⌘L, colour flags), junk (⇧⌘J: moves to Junk, marks sender; "Not Junk" back), New Mailbox,
  rename, delete mailbox. Undo for move/delete/flag/junk.
- Search: field in the toolbar; tokens for From/To/Subject; scope "All Mailboxes" vs current;
  local index first, then server search for older mail.
- Attachments: Quick Look (Space) through `rmac-quick-look`, Save (to Downloads by default),
  open with the default app via the portal; inline images.
- Notifications: one banner per new Inbox message while Mail runs (sender, subject, preview),
  grouped by Lulo's notification service, Dock badge with the unread Inbox count; clicking opens
  the message. Respects Focus.
- Junk: server Junk mailbox is honoured; local filter "Mark as Junk Mail but leave it in my
  Inbox / Move it to the Junk mailbox" (rules on headers, sender history, server spam flags).
- Signatures: per account, rich text, chosen automatically, editable in Settings ▸ Signatures.
- Settings: General (default mail reader — wires S16, new-message notifications, sound,
  downloads folder, remove unedited downloads), Accounts (account info, mailbox behaviours,
  server settings read-only for OAuth accounts), Junk Mail, Viewing (list preview lines, show
  To/Cc, load remote content), Composing (format, reply quoting, check spelling), Signatures,
  Privacy ("Block All Remote Content"). **(verify on Mac: pane list and labels)**
- `mailto:` handler and "Share ▸ Mail" from Files/Preview open a compose window.

**Mac behaviour to match (verify on Mac):** macOS 26 sidebar is a floating glass panel; message
list rows ~ 70 pt with a 2-line preview; unread dot blue at the left; VIP star; Mail Categories
(Primary/Transactions/Updates/Promotions) toggle; Protect Mail Activity; "Remind Me", "Send
Later", "Unsend" (10 s default) in the compose window; summaries.

**Deferred:** Mail Categories and summaries (on-device ML), Rules, Smart Mailboxes, VIPs,
Remind Me, Send Later, Unsend (cheap with the Outbox: candidate for Beta 1.1), S/MIME/PGP,
Exchange EWS, POP3, stationery, printing, Hide My Email, Mail Privacy Protection proxying
(Lulo blocks remote content instead).

## 4. Design notes from the mocks

- Window radius 27, toolbar 52, sidebar floating panel inset 8 with radius 19 — same as Files
  (`design-lab/finder.html`). Calendar sidebar 220 wide; Mail sidebar 220, list 340.
- Calendar colours are the system palette tokens (`--sys-red`, `--sys-orange`…), events use the
  colour at 22 % (dark) / 18 % (light) fill, solid 3 pt left edge, 11/12 pt semibold title.
- Today and the now-line use `--sys-red`; selection uses the accent.
- Compose window 640 × 560, borderless header fields separated by hairlines.

## 5. Build plan

Sizes: **S** ≤ 2 agent-days, **M** 3–5, **L** 6–10. Each milestone ends with laptop
`cargo test -p <crate>` + clippy + fmt, a parity row update, and the listed scenarios.
Rules for every milestone: no new `gpui_component` imports (use `rmac-ui`, ADR 0015), design
tokens only, no polling, no UI-thread I/O, HTML mock first for any new surface.

### Shared accounts (ACC)

| # | Milestone | Size | Crates | Tests / scenarios |
|---|---|---|---|---|
| ACC-1 | Domain: providers table (GOA IDs, client IDs, scopes, server presets), account/service model, autoconfig decision logic, sign-in state machine | M | `rmac-accounts` (no D-Bus) | unit tests: domain→provider, ISPDB XML parse, state machine; laptop test compares the provider table with `strings libgoa-backend` |
| ACC-2 | GOA adapter: ObjectManager watch, `AddAccount`, `Remove`, service toggles, token/password fetch API, OAuth2 handler name owner + PKCE + code exchange, autoconfig HTTP/DNS | L | `rmac-accounts-linux` (zbus, `ureq`+rustls) | fake goa-daemon on a private bus (python-dbusmock); no secret in logs test |
| ACC-3 | Internet Accounts pane + Add Account sheet (from `design-lab/internet-accounts.html`) | M | `system-settings` (`controller/internet_accounts`), `rmac-accounts-ui` | `docs/behavior-pending/settings/internet-accounts-add-sheet.json`; Orca assert script; SET-63 |
| ACC-4 | Session packaging: deps/Recommends (`evolution-ews`), `calendar-agent` unit, `mailto`/`text/calendar` MIME, desktop files, original app icons (no Apple artwork) | S | `packaging/`, `assets/icons` | `scripts/test_session_package.py` additions |

### Calendar (CAL)

| # | Milestone | Size | Crates | Tests / scenarios |
|---|---|---|---|---|
| CAL-1 | Store: iCalendar parse/serialise, RRULE/EXDATE/RECURRENCE-ID expansion, time zones, overlap layout for day/week columns | L | `rmac-calendar-store` | property tests on recurrence vs RFC 5545 examples; layout golden tests |
| CAL-2 | EDS adapter: sources, open calendar, views, create/modify/remove, receive/send objects, refresh, offline state | L | `rmac-calendar-eds`, `rmac-calendar-runtime` | integration test against a private `evolution-source-registry` + local file calendar on the laptop (temporary XDG dirs) |
| CAL-3 | App shell: window, toolbar, sidebar (calendar list, mini month), menus (`rmac-app-menu` `CALENDAR_MENUS`), Week view read-only | M | `calendar` | `calendar/switch-views`, `calendar/go-to-today` |
| CAL-4 | Day, Month, Year views, keyboard navigation, scrolling | M | `calendar` | `calendar/month-keyboard-navigation`; idle CPU sample |
| CAL-5 | Create/edit/move/resize/delete, inspector popover, repeat-edit choices, undo | L | `calendar` | `calendar/new-event`, `calendar/edit-event-inspector`, `calendar/delete-event-undo` |
| CAL-6 | Calendars management, subscriptions, import/export, search, Settings window | M | `calendar` | `calendar/hide-calendar`, `calendar/search-events` |
| CAL-7 | Reminders agent + notification actions + default alerts | M | `calendar-agent` | timerfd unit tests with a fake clock; laptop alert-on-time check; 0 wakeups check (ADR 0022 §6) |
| CAL-8 | Invitations (inbox popover, accept/decline), menu-bar clock & desktop widget integration, Orca pass | M | `calendar`, `rmac-desktop-widgets`, top bar | `calendar/invitation-accept` (fixture CalDAV server) |
| CAL-9 | Microsoft calendars: verify `evolution-ews` Microsoft 365 backend with GOA `ms_graph`; if unusable, Graph calendar adapter behind the same runtime trait | M | `rmac-calendar-eds` or `rmac-calendar-graph` | laptop check with the owner's test account |

### Mail (MAIL)

| # | Milestone | Size | Crates | Tests / scenarios |
|---|---|---|---|---|
| MAIL-1 | Storage: SQLite schema + migrations, blob store, FTS5 search, JWZ threading, offline change journal | M | `rmac-mail-store`, `rmac-mail-storage` | unit tests; crash-safety test (kill during write) |
| MAIL-2 | IMAP client: TLS, SASL (XOAUTH2/PLAIN), CONDSTORE/QRESYNC sync, IDLE, MOVE/UIDPLUS, SPECIAL-USE; dependency review | L | `rmac-mail-imap` | against an in-tree Rust fixture IMAP server (no Docker, per AGENTS.md) |
| MAIL-3 | SMTP + Outbox + MIME build/parse, reply/forward quoting, HTML sanitiser → rich-text model | L | `rmac-mail-smtp`, `rmac-mail-mime` | sanitiser corpus tests (scripts, trackers, CSS url()), quoting golden tests |
| MAIL-4 | Sync runtime: per-account worker, account discovery from GOA, connectivity back-off, notifications, unread badge | M | `rmac-mail-runtime` | fake GOA + fixture server; idle wakeup budget test |
| MAIL-5 | App shell: three-pane window, sidebar, message list (virtual list), viewer (rich-text renderer), menus (`MAIL_MENUS`) | L | `mail` | `mail/open-message-marks-read`, `mail/toggle-threads` |
| MAIL-6 | Compose window: fields, address completion (EDS contacts), attachments, signatures, drafts, send | L | `mail` (reuses `rmac-editor`) | `mail/compose-new-message`, `mail/reply-quotes`, `mail/forward` |
| MAIL-7 | Organise + search: delete/archive/move/flag/junk, undo, search tokens, server search | M | `mail` | `mail/flag-message`, `mail/move-to-junk`, `mail/delete-and-undo`, `mail/search-mailbox` |
| MAIL-8 | Settings window, signatures editor, default mail app (S16), `mailto:`, `.ics` invitations → Calendar, iMIP replies | M | `mail`, `system-settings` | `mail/settings-signature` |
| MAIL-9 | Microsoft Graph backend (messages, delta, sendMail, folders) | M | `rmac-mail-graph` | recorded-response tests; laptop check with a test account |
| MAIL-10 | Accessibility + performance pass: Orca script, 100–200 % scaling, 10 k-message mailbox scroll at 60 fps on the laptop, memory soak | M | `mail`, `scripts/assert_mail_accessibility.py` | `scripts/behavior/run_memory_soak.py` entry |

Order: ACC-1 → ACC-2 → (ACC-3 ∥ CAL-1 ∥ MAIL-1) → CAL-2 ∥ MAIL-2 → … Calendar is ~9 milestones
(≈ 6 agent-weeks), Mail ~10 (≈ 8 agent-weeks), accounts 4 (≈ 3 agent-weeks). Calendar's
critical path is CAL-1/CAL-2; Mail's is MAIL-2/MAIL-3/MAIL-5.

## 6. Risks and blockers

1. **Using GNOME's OAuth clients** from Lulo's own UI (ADR 0022 §2): owner approval needed; the
   consent screen says "GNOME".
2. **Microsoft personal accounts** with GOA `ms_graph` (tenant `common`) are unverified; mail must
   use Graph, not IMAP; calendars need `evolution-ews` (not installed by default).
3. **Yahoo and iCloud** have no GOA OAuth provider: app-specific passwords only in Beta 1 (the
   Mac signs in with the provider's web page).
4. **HTML mail rendering** without a web engine: the rich-text model will not match complex
   newsletters; "Open in Browser" is the escape hatch. This is the largest visual-parity risk.
5. **EDS D-Bus API is semi-private** and versioned; an Ubuntu point release could rename it.
6. **Resident memory when accounts exist:** goa-daemon + goa-identity-service + EDS registry +
   calendar factory ≈ 34 MiB RSS while Calendar or the agent is in use.
7. **Mac recordings** for these scenarios must not touch the owner's synced data: use a local
   "On My Mac" calendar and a local mailbox only, never send, never confirm deletions.
