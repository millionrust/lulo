"""polkitd (org.freedesktop.PolicyKit1.Authority) python-dbusmock template.

Used by crates/rmac-polkit-agent/tests/authority_mock.rs to prove Lulo's
polkit authentication agent registers for the graphical session and
answers BeginAuthentication and CancelAuthentication the way polkitd
drives a real agent. It models only the agent side of the authority:

* RegisterAuthenticationAgentWithOptions / RegisterAuthenticationAgent /
  UnregisterAuthenticationAgent record (subject kind, session id, locale,
  object path); GetRegistrations (mock interface) returns them.
* Begin(agent, path, action, message, icon, details, cookie, uids) (mock
  interface) calls the agent's BeginAuthentication from this process, the
  authority's own connection, just as polkitd does, with unix-user
  identities for `uids`; the reply is recorded for GetResult(cookie): ""
  while pending, "ok", or the D-Bus error name.
* Cancel(agent, path, cookie) calls CancelAuthentication.

Never touches the real system bus: it runs on the private bus the test
starts.
"""

import dbus

import dbus.service

from dbusmock import MOCK_IFACE

BUS_NAME = "org.freedesktop.PolicyKit1"
MAIN_OBJ = "/org/freedesktop/PolicyKit1/Authority"
MAIN_IFACE = "org.freedesktop.PolicyKit1.Authority"
AGENT_IFACE = "org.freedesktop.PolicyKit1.AuthenticationAgent"
SYSTEM_BUS = True

STATE = {"registrations": [], "results": {}}


def _record(subject, locale, path):
    kind, details = subject
    session = str(details.get("session-id", ""))
    STATE["registrations"].append((str(kind), session, str(locale), str(path)))


def _register_with_options(self, subject, locale, path, options):
    _record(subject, locale, path)


def _register(self, subject, locale, path):
    _record(subject, locale, path)


def _unregister(self, subject, path):
    STATE["registrations"] = [
        entry for entry in STATE["registrations"] if entry[3] != str(path)
    ]


@dbus.service.method(MOCK_IFACE, in_signature="", out_signature="a(ssss)")
def GetRegistrations(self):
    return dbus.Array(
        [dbus.Struct(entry, signature="ssss") for entry in STATE["registrations"]],
        signature="(ssss)",
    )


@dbus.service.method(MOCK_IFACE, in_signature="sssssa{ss}sau", out_signature="")
def Begin(self, agent, path, action_id, message, icon, details, cookie, uids):
    cookie = str(cookie)
    STATE["results"][cookie] = ""
    identities = dbus.Array(
        [
            dbus.Struct(
                ("unix-user", dbus.Dictionary({"uid": dbus.UInt32(uid)}, signature="sv")),
                signature="sa{sv}",
            )
            for uid in uids
        ],
        signature="(sa{sv})",
    )

    def done():
        STATE["results"][cookie] = "ok"

    def failed(error):
        STATE["results"][cookie] = error.get_dbus_name() or "error"

    self.connection.call_async(
        str(agent),
        str(path),
        AGENT_IFACE,
        "BeginAuthentication",
        "sssa{ss}sa(sa{sv})",
        (
            action_id,
            message,
            icon,
            dbus.Dictionary(details, signature="ss"),
            cookie,
            identities,
        ),
        done,
        failed,
        timeout=600.0,
    )


@dbus.service.method(MOCK_IFACE, in_signature="sss", out_signature="")
def Cancel(self, agent, path, cookie):
    def ignore(*_):
        pass

    self.connection.call_async(
        str(agent), str(path), AGENT_IFACE, "CancelAuthentication", "s", (cookie,), ignore, ignore
    )


@dbus.service.method(MOCK_IFACE, in_signature="s", out_signature="s")
def GetResult(self, cookie):
    return STATE["results"].get(str(cookie), "")


def load(mock, parameters):
    STATE["registrations"] = []
    STATE["results"] = {}
    mock.AddMethods(
        MAIN_IFACE,
        [
            ("RegisterAuthenticationAgentWithOptions", "(sa{sv})ssa{sv}", "", _register_with_options),
            ("RegisterAuthenticationAgent", "(sa{sv})ss", "", _register),
            ("UnregisterAuthenticationAgent", "(sa{sv})s", "", _unregister),
        ],
    )
