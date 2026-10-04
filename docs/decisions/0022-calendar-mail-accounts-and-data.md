# ADR 0022 — Calendar and Mail: GOA holds the accounts, EDS serves calendars, Mail has its own Rust engine

- **Status:** proposed 2026-10-03. Needs owner approval of decision 2 (Lulo's own sign-in UI
  using GNOME's registered OAuth clients) before milestone ACC-2 starts.
- **Scope:** new crates `rmac-accounts`, `rmac-accounts-linux`, `rmac-accounts-ui`,
  `rmac-calendar-*`, `calendar`, `calendar-agent`, `rmac-mail-*`, `mail`; a new System
  Settings pane `crates/system-settings/src/controller/internet_accounts`; session packaging.
- **Closes (when built):** APP-01 (Calendar), APP-13 (Mail), SET-63 (Internet Accounts).
- **Product scope and milestones:** `docs/design/calendar-mail.md`. Mocks:
  `design-lab/calendar.html`, `design-lab/mail.html`, `design-lab/internet-accounts.html`.

## The question

Beta 1 must ship a Calendar and a Mail app modelled on macOS 26, and signing in must feel like
the Mac: pick Google, Microsoft, Yahoo, iCloud or Other, sign in, done. Lulo targets low-spec
PCs: no polling, idle CPU about zero, and as little resident memory as possible. Which parts do
we reuse from Ubuntu, and which do we write?

## What was measured (reference laptop, read-only, 2026-10-03)

Ubuntu 26.04 LTS has, installed by default:

| Component | Version | What matters |
|---|---|---|
| `gnome-online-accounts` (goa-daemon) | 3.58.0 | Providers: `google`, `ms_graph` ("Microsoft 365: email, calendars, contacts and files"), `exchange` (EWS), `owncloud` (Nextcloud), `webdav` (CalDAV/CardDAV), `imap_smtp`, `kerberos`, `fedora`. **No Yahoo, no iCloud, no `windows_live`.** |
| GOA D-Bus | — | `org.gnome.OnlineAccounts.Manager.AddAccount(sssa{sv}a{ss}) → o`, `IsSupportedProvider(s)`; accounts under `/org/gnome/OnlineAccounts/Accounts/*` via ObjectManager. The owner has no accounts. |
| GOA OAuth2 | — | Authorization-code + PKCE in the system browser. Google client `44438659992-7kgj…apps.googleusercontent.com`; Microsoft client `b155a604-…` with Graph scopes only (`mail.readwrite mail.send calendars.readwrite …`, no IMAP scope). Credentials stored in the keyring as `access_token`, `access_token_expires_at`, `refresh_token`. |
| `goa-oauth2-handler` | — | Registered for `x-scheme-handler/goa-oauth2` and the Google client's reverse-DNS scheme. It forwards the redirect to `org.gnome.OnlineAccounts.OAuth2.Response(s client_id, s uri)` at `/org/gnome/OnlineAccounts/OAuth2` on the session bus. Whoever owns that name during sign-in receives the code. |
| GOA IMAP autoconfig | — | `imap_smtp` already knows Mozilla ISPDB (`v1.ispdb.net`) and `autoconfig.<domain>` lookups, but only inside its GTK add-account UI. |
| `evolution-data-server` | 3.56.2 | Calendar backends: CalDAV, ICS-over-HTTP (subscriptions), local file, Google Tasks, weather; registry modules for GOA, Google, Outlook, Yahoo, WebDAV. `CalendarFactory.OpenCalendar(s) → (ss)`. **No Microsoft 365/EWS calendar backend** without `evolution-ews` (3.56.2-3 is in the archive, not installed). |
| Resident cost when active | — | goa-daemon 10 MiB, goa-identity-service 6.5, source-registry 9, calendar-factory 8, addressbook-factory 9.6, alarm-notify 10.5 (RSS, live session). |
| Browser | — | Firefox (snap) is the only browser. |

`docs/parity.md` MEM-02 skips `evolution-alarm-notify` in the Lulo session; everything else
above is D-Bus activated, so it costs nothing until something asks for it.

## What the Mac does

- Accounts live in one place: System Settings ▸ Internet Accounts. Mail ▸ Add Account… and
  Calendar ▸ Add Account… open the same provider chooser (iCloud, Microsoft Exchange, Google,
  Yahoo!, AOL, Other). Google/Microsoft/Yahoo sign in through a web sheet; Other asks for name,
  address and password and auto-discovers servers. Each account lists the services it syncs
  (Mail, Contacts, Calendars, Notes…) as switches. *(Verify on Mac: whether macOS 26's sheet
  starts with an e-mail field, as SET-63 records, or with the provider list.)*
- Calendar alerts are delivered by a background agent even when Calendar is closed.
- Mail notifications arrive only while Mail is running (it may be hidden). *(Verify on Mac.)*
- Mail syncs IMAP with push where offered (iCloud, IDLE) and periodic fetch otherwise.

## Decision

1. **GNOME Online Accounts is the only account store.** Lulo never stores a password or token
   itself. Apps read accounts through GOA's ObjectManager (`InterfacesAdded/Removed`,
   `PropertiesChanged`) and ask for credentials at use time with
   `OAuth2Based.GetAccessToken` or `PasswordBased.GetPassword`. GOA refreshes tokens and keeps
   them in the login keyring (Secret Service). EDS's GOA module turns every GOA account with
   `CalendarEnabled` into calendar sources automatically.

2. **Lulo draws its own sign-in UI and runs GOA's own OAuth flow.** The Internet Accounts pane
   (and the same sheet in Mail and Calendar) does what libgoa-backend's GTK dialog does:
   - build GOA's authorization URL with GOA's client ID, scopes and redirect URI and a fresh
     PKCE verifier and `state`;
   - own `org.gnome.OnlineAccounts.OAuth2` for the duration of the sign-in only (queue
     `DoNotQueue`; if GNOME Settings already owns it, say "Finish signing in in the other window"
     and stop);
   - open the URL in the default browser through the OpenURI portal (`rmac-portal::open_uri`);
   - receive `Response(client_id, uri)` from `goa-oauth2-handler`, check `state`, exchange the
     code over TLS, read the identity (Google `userinfo`, Graph `/me`);
   - call `Manager.AddAccount(provider, identity, presentation_identity, credentials, details)`
     with the same credential keys GOA stores, so goa-daemon can refresh the token later.
   Because the client IDs are GNOME's, Lulo needs no Google restricted-scope verification and no
   Microsoft app registration for Beta 1. **Consequences the owner must accept:** the consent
   screen says "GNOME", and if GNOME changes a client ID or scope list in a GOA update, Lulo's
   copy must follow (the provider table is data in `rmac-accounts`, checked against the installed
   `libgoa-backend` by a laptop test). Registering Lulo's own clients is a post-Beta task
   (Google requires a paid CASA assessment for `https://mail.google.com/`).
   Rejected: shelling out to `gnome-control-center online-accounts` (GNOME look, ~60 MiB, may not
   run outside GNOME) and linking libgoa-backend in a GTK helper (same look problem).

3. **Provider mapping.**

   | Button | GOA accounts created | Mail | Calendar |
   |---|---|---|---|
   | Google | `google` (OAuth) | IMAP/SMTP, SASL XOAUTH2 | EDS CalDAV (GOA source) |
   | Microsoft (Outlook, Hotmail, Microsoft 365) | `ms_graph` (OAuth, tenant `common`) | Microsoft Graph REST (the token has no IMAP scope) | EDS's GOA module maps `ms_graph` to the `microsoft365` backend from `evolution-ews-core` (CAL-9, see `docs/design/calendar-mail.md`) |
   | Yahoo | `imap_smtp` + `webdav` (app password) | IMAP/SMTP, PLAIN | EDS CalDAV `caldav.calendar.yahoo.com` |
   | iCloud | `imap_smtp` + `webdav` (app-specific password) | IMAP/SMTP, PLAIN | EDS CalDAV `caldav.icloud.com` |
   | Other | `imap_smtp` after autoconfig; optional `webdav` | IMAP/SMTP | CalDAV if discovered (RFC 6764) |

   Yahoo and iCloud have no OAuth provider in GOA, so their sheet explains app-specific passwords
   with a button that opens the provider's page. A Lulo account that spans two GOA accounts is
   grouped by a small file, `~/.config/lulo/accounts.json` (GOA object IDs only, mode 0600).
   **Autoconfig** runs in `rmac-accounts-linux`: built-in table → `autoconfig.<domain>` →
   ISPDB → MX lookup (an MX at Google or Outlook switches to that provider's OAuth sheet) →
   manual server fields. *(Verify on the laptop with a real outlook.com account that `ms_graph`
   accepts personal accounts with tenant `common`.)*

4. **Calendars come from evolution-data-server over D-Bus.** `rmac-calendar-eds` speaks
   `Sources5` (ObjectManager), `CalendarFactory.OpenCalendar`, the `Calendar` object and live
   views (`ObjectsAdded/Modified/Removed`), using zbus 5 (already in the workspace). Lulo never
   links libecal (LGPL, GObject). iCalendar parsing, recurrence expansion and time zones are pure
   Rust in `rmac-calendar-store` (no GPUI, no D-Bus). EDS gives us CalDAV sync, Google, ICS
   subscriptions, the local "On My Computer" calendar, an offline cache and a write-back queue
   for free. The D-Bus API is versioned (`Sources5`, `Calendar8`); the adapter checks the names
   at start-up and fails with a clear "Calendar service unavailable" state if they change.
   Rejected: CalDAV directly in Rust (re-implements EDS: sync tokens, scheduling, caching,
   Google quirks; ~3 extra agent-months).

5. **Mail gets its own Rust engine, inside the Mail process.** No suitable engine exists: EDS
   does not serve mail over D-Bus (Camel is an in-process C library), Geary's engine is not a
   library, and GPL/LGPL-only crates are barred by `docs/dependency-policy.md`. The engine:
   - `rmac-mail-imap`: IMAP4rev1/rev2 on rustls (already locked), CONDSTORE/QRESYNC, UIDPLUS,
     MOVE, IDLE, SPECIAL-USE, COMPRESS optional; SASL XOAUTH2/OAUTHBEARER, PLAIN, LOGIN. Built
     on a permissively licensed codec (candidates `imap-codec`/`imap-next`, Apache-2.0/MIT;
     dependency review in milestone MAIL-2).
   - `rmac-mail-graph`: Microsoft Graph messages, `delta` sync and `sendMail`.
   - `rmac-mail-smtp`: SMTP submission (587 STARTTLS / 465 TLS), XOAUTH2 or PLAIN.
   - `rmac-mail-mime`: parse (`mail-parser`), build (`mail-builder`), quote, forward; HTML is
     sanitised into a small rich-text model that GPUI draws (paragraphs, inline styles, links,
     lists, quotes, simple tables, images). Remote images stay blocked until "Load Remote
     Content". Messages the model cannot draw faithfully offer "Open in Browser" (a sanitised
     copy in a private temp file).
   - `rmac-mail-storage`: SQLite (`rusqlite`, bundled, FTS5) for mailboxes, headers, flags,
     threads and the search index; bodies and attachments as files named by content hash.
   Rejected: Thunderbird as the Mail app (not Mac-like, ~250 MiB), linking Camel (LGPL C,
   GObject threading, no Rust story).

6. **No polling; idle cost is budgeted.**

   | State | Budget | How |
   |---|---|---|
   | Calendar open, idle | 0 wakeups from Lulo | EDS view signals; one timer for the red now-line, re-armed once a minute only while a Day/Week view of today is visible and the window is not occluded |
   | Calendar closed | `calendar-agent` ≤ 4 MiB RSS, 0 wakeups between alerts | one `timerfd` (`CLOCK_REALTIME`, `TFD_TIMER_CANCEL_ON_SET`) to the next alert, re-armed on EDS signals, clock changes and resume |
   | Mail open, idle | ≤ 1 wakeup/min per account, ≤ 0.1 % CPU | one IMAP IDLE connection per account on Inbox (re-issued every 25 min, RFC 2177); other mailboxes refreshed when selected, on window focus and every 15 min; Graph accounts `delta` every 5 min (Graph has no desktop push) |
   | Mail closed | nothing | no daemon, matching the Mac |
   | EDS | its own refresh | CalDAV sources refresh on EDS's interval (exposed as Calendar ▸ Settings ▸ Accounts ▸ Refresh Calendars); the network monitor triggers a refresh on reconnect |

   All network and disk work runs on worker threads; the GPUI thread only receives snapshots.

7. **Calendar reminders use a small Lulo agent, not `evolution-alarm-notify`.** MEM-02 stays.
   `calendar-agent` (no GPUI, zbus + timerfd) is a systemd user service with
   `ConditionPathExists=%h/.local/state/lulo/calendar/agent-enabled`; Calendar creates the
   marker the first time a calendar or an event with an alert exists, and starts the unit. It
   reads alarms through EDS views (`has-alarms-in-range?`), expands recurrences with
   `rmac-calendar-store`, and posts through `org.freedesktop.Notifications` (Lulo's
   notification service) with actions *Snooze* (5 min / 15 min / 1 h / tomorrow, verify on Mac)
   and *Close*; the default action opens the event in Calendar. Fired/snoozed state lives in
   `~/.local/state/lulo/calendar/alerts.json`. Missed alerts after suspend are shown once, newest
   first, as the Mac does. Cost: ~4 MiB, versus ~10.5 MiB plus a GTK reminders window for
   `evolution-alarm-notify`.

8. **Offline behaviour.** Calendar: EDS serves its cache and queues writes; the toolbar shows an
   offline badge per account (Mac shows a warning triangle next to the account in the sidebar).
   Mail: reading, searching and composing work from the cache; sending goes to an Outbox
   (SQLite) that drains when the network returns; flag/move/delete changes are journalled and
   replayed with UID checks. `rmac-network`'s connectivity state (NetworkManager signals) drives
   reconnects with exponential back-off (1 s → 5 min), never a fixed poll.

9. **Storage.**
   - Mail: `~/.local/share/lulo/mail/<account-uuid>/{index.sqlite3,blobs/}`, directories 0700,
     files 0600. Default offline policy for low-spec disks: all headers; bodies of the last 90
     days and anything opened; attachments on demand. Settings ▸ Accounts offers "All messages".
     *(Mac downloads everything by default.)*
   - Calendar: EDS's cache (`~/.cache/evolution/calendar`); Lulo keeps only UI state
     (selected view, hidden calendars) in `~/.config/lulo/calendar.json`.
   - Removing an account deletes its Lulo cache and its GOA account(s); the sheet says so.

10. **Security.**
    - TLS is mandatory for IMAP, SMTP, Graph, CalDAV and autoconfig; certificates are verified
      against the system store (`rustls-native-certs`). No plaintext fallback; STARTTLS failures
      are errors. An untrusted certificate is refused in Beta 1 (the Mac offers "Continue";
      deferred).
    - Tokens, passwords, authorization codes, message bodies, subjects and addresses are never
      logged. Credential types are wrapped in a `Secret` newtype whose `Debug`/`Display` print
      `[redacted]`, following `rmac-notifications`' redaction rule. A unit test greps log output
      in the engine's integration tests for planted secrets.
    - OAuth: PKCE S256, random `state`, redirect accepted only from the name owner's
      `Response` call while a sign-in is pending; the name is released on finish/cancel.
    - HTML mail: scripts, forms, iframes, event attributes and CSS `url()` are stripped; links
      show their real target on hover and open through the portal; remote content is blocked by
      default per message and per sender.
    - The SQLite cache is not encrypted (same as the Mac's `~/Library/Mail`); it relies on file
      permissions and disk encryption.

11. **Accessibility.** Every view must pass an Orca check script like
    `scripts/assert_notes_accessibility.py`:
    - Calendar grid is a `table` with `table cell`s named "Thursday 8 October, 3 events"; each
      event is a `push button` named "Team sync, 10:00 to 10:30, Work calendar, has invitees".
      Arrow keys move the selected day, Return opens the event inspector, Space/Delete as Mac.
    - Mail message list is a `table` whose rows are named "Unread, flagged, Anna Kim, Lunch on
      Friday, 10:14, 1 attachment"; the reader is a `document frame`; compose fields are labelled
      `entry`s. New mail is announced only through the notification, never by focus stealing.
    - Reduced motion removes sheet and inspector animations; colours come from `rmac-design`
      tokens and calendar colours keep ≥ 3:1 contrast against the event text in both themes.

## Consequences

- Packaging adds `Recommends: evolution-ews-core` to `rmac-apps` (Microsoft calendars, CAL-9;
  the `evolution-ews` package itself would pull in the Evolution client) and keeps
  `gnome-online-accounts` and `evolution-data-server` as dependencies (they are already
  installed on stock Ubuntu). If `evolution-ews-core` is absent, Microsoft accounts show Mail only and
  the Calendars switch explains why.
- New third-party crates need the usual cargo-deny review: an IMAP codec, `mail-parser`,
  `mail-builder`, `rusqlite` (bundled SQLite, public domain), an iCalendar parser, `rrule`,
  `chrono-tz`, an HTML tokenizer (`html5ever` is already locked), a small HTTPS client on rustls
  (`ureq`). Each is Apache-2.0/MIT or more permissive; licences are verified in the milestone
  that adds them.
- `scripts/behavior/scenario.py` gains `calendar` and `mail` apps and facts
  (`calendar_events`, `mail_messages`), and the Mac recorder must only touch an "On My Mac"
  calendar and a local mailbox: it never sends mail and never changes a synced account.
- The OAuth sign-in cannot run in CI. Laptop checks use a disposable test account the owner signs
  in once; automated tests use a local IMAP/SMTP/CalDAV fixture server and a fake goa-daemon on a
  private session bus.

## How to verify on the reference laptop

- `busctl --user introspect org.gnome.OnlineAccounts /org/gnome/OnlineAccounts/Manager` shows
  `AddAccount`; after ACC-2, `busctl --user tree org.gnome.OnlineAccounts` lists the new
  account and `secret-tool search --all goa-identity …` returns one item (never print it).
- With Calendar closed and one event alert 2 minutes ahead, `calendar-agent` shows 0 wakeups in
  `perf stat -e 'sched:sched_wakeup'` until the alert, and a banner appears on time.
- Mail idle for 10 minutes with one IDLE account: `scripts/behavior/measure_memory.py` and
  `pidstat 60` show ≤ 0.1 % CPU.
