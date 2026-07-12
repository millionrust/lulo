# ADR 0004: secure session-lock boundary

- Status: Accepted
- Date: 2026-07-12

## Context

A normal GPUI window, fullscreen surface, or layer-shell overlay cannot secure a
Wayland session. Other clients could remain visible or receive input, a crashed
overlay would reveal the desktop, and the UI process would have to handle the
user's authentication secret. Calling logind's `LockSession` alone is also not
a lock: logind emits a request that a session manager must honor.

The Wayland `ext-session-lock-v1` protocol makes the compositor the security
boundary. After its `locked` event, normal clients are hidden and receive no
input; if the locker disappears, the compositor must remain locked rather than
reveal the session. The locker supplies one surface for every current and newly
connected output and is responsible for authentication.

The initial threat model protects an unattended active session from a physical
user and prevents ordinary Wayland clients from observing or receiving lock
input. It does not claim to contain an already-compromised process running as
the same Unix user, a malicious PAM stack, root, kernel compromise, DMA attacks,
or an unlocked secondary TTY/session.

## Decision

1. niri plus `ext-session-lock-v1` owns exclusion of normal content and input.
   No rmac GPUI surface is ever described or shipped as a secure lock.
2. Ubuntu's signed, PAM-enabled `/usr/bin/swaylock` is the initial authentication
   and multi-output provider. rmac never reads, stores, logs, or transports the
   password. A future original rmac locker must pass a separate security review
   and use the same compositor protocol and PAM boundary before replacing it.
3. `rmac-locker` starts swaylock in the foreground with `--ready-fd=1`. It tells
   systemd that `rmac-lock.service` is ready only after swaylock writes its
   readiness newline, which swaylock defines as the point where the compositor
   guarantees that no security-sensitive content is visible.
4. A successful swaylock exit is treated as authenticated unlock. Any nonzero
   exit leaves logind's locked hint set and makes the unit fail; systemd restarts
   it without a start limit while niri keeps the session on its locked fallback
   background. The entire control group is killed together on service stop.
5. The supervisor updates logind's `LockedHint` after compositor readiness and
   after successful unlock. This hint is advisory state for the wider desktop;
   it is never used as proof that the compositor is locked.
6. The direct shortcut and generated niri fallback start only the fixed
   `rmac-lock.service`. The fallback lock binding is allowed while locked so a
   user can restart the provider from niri's red recovery screen.
7. The default appearance is an original quiet dark rmac treatment. It contains
   no notification content, screenshots, user files, or network resources. A
   user-owned config is installed only when absent and is never overwritten.
8. The bootstrap imports `XDG_SESSION_ID` through its routing-only allow-list.
   `rmac-lock-coordinator` asks logind to resolve that exact session object,
   listens for that session's `Lock()` signal, and holds a delay sleep
   inhibitor. On `PrepareForSleep(true)` it retains the inhibitor until the lock
   unit's readiness transaction succeeds; after resume it reacquires a fresh
   inhibitor. It deliberately ignores logind `Unlock()` requests because PAM
   is the only unlock authority.
9. Before E5 can be complete, lock-before-suspend, resume, lid close,
   multi-monitor hotplug, PAM, failure recovery, and emergency TTY recovery
   require Linux evidence.
10. `/usr/bin/swayidle` supplies compositor idle detection only. A bounded,
    versioned policy selects a timeout or Never, while rmac constructs the fixed
    readiness-gated lock command. logind remains authoritative for lid and
    suspend operations so two policy engines cannot race to suspend the host.

## Consequences

- Security remains correct even though the first visual provider cannot yet
  reproduce every macOS lock-screen detail.
- The shell can crash or enter safe mode without becoming the authentication
  boundary.
- The distro package and PAM configuration become explicit installation and
  hardware-matrix dependencies.
- Lock Screen Settings must hide idle/suspend/preview controls until the delay
  inhibitor and authoritative settings service exist.

## Rejected alternatives

- GPUI fullscreen or layer-shell overlay: cosmetic and fail-open.
- `loginctl lock-session` without a listener: request only, no compositor lock.
- Bundling or forking a locker immediately: expands the authentication attack
  surface before protocol, PAM, crash, and hotplug evidence exists.
- Unlocking when the locker crashes: violates fail-closed session locking.

## Upstream contracts

- `ext-session-lock-v1`: <https://wayland.app/protocols/ext-session-lock-v1>
- systemd-logind D-Bus API:
  <https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.login1.html>
- swaylock source and release policy: <https://github.com/swaywm/swaylock>
- swaylock readiness/configuration contract:
  <https://github.com/swaywm/swaylock/blob/master/swaylock.1.scd>
- swayidle event and wait contract:
  <https://github.com/swaywm/swayidle/blob/master/swayidle.1.scd>
- niri locker recovery guidance: <https://niri-wm.github.io/niri/FAQ.html>
