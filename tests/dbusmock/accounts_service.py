"""AccountsService (org.freedesktop.Accounts) python-dbusmock template.

Used by crates/rmac-users-linux/tests/accountsservice_mock.rs and by
scripts/behavior/fake_hardware.py so System Settings ▸ Users & Groups has
real D-Bus data in tests. It models the parts Lulo calls: listing users,
reading their properties, the per-user setters, CreateUser and DeleteUser,
and polkit refusing a change (SetAuthorized(False) makes every mutation
raise org.freedesktop.Accounts.Error.PermissionDenied, as AccountsService
does when polkit says no).

Parameters (JSON): {"users": [{"uid": 1000, "name": "jacob",
"real_name": "Jacob Samas", "admin": true}, ...]}.

The mock never stores a plain-text password: SetPassword keeps only the
crypted hash it was given, in the mock-only MockPasswordHash property, so
tests can assert that what crossed the bus was a hash.
"""

import dbus

from dbusmock import MOCK_IFACE, mockobject

BUS_NAME = "org.freedesktop.Accounts"
MAIN_OBJ = "/org/freedesktop/Accounts"
MAIN_IFACE = "org.freedesktop.Accounts"
USER_IFACE = "org.freedesktop.Accounts.User"
SYSTEM_BUS = True

STATE = {"authorized": True, "deleted": []}

DEFAULT_USERS = [
    {"uid": 1000, "name": "jacob", "real_name": "Jacob Samas", "admin": True},
    {"uid": 1001, "name": "amy", "real_name": "Amy Brown", "admin": False},
]


def _main():
    return mockobject.objects[MAIN_OBJ]


def _require_authorization():
    if not STATE["authorized"]:
        raise dbus.exceptions.DBusException(
            "Not authorized", name="org.freedesktop.Accounts.Error.PermissionDenied"
        )


def _user_path(uid):
    return f"/org/freedesktop/Accounts/User{int(uid)}"


def _users():
    return sorted(path for path in mockobject.objects if path.startswith("/org/freedesktop/Accounts/User"))


def _setter(prop):
    def set_value(self, value):
        _require_authorization()
        self.Set(USER_IFACE, prop, value)
        self.EmitSignal(USER_IFACE, "Changed", "", [])

    return set_value


def _set_password(self, crypted, hint):
    _require_authorization()
    if not str(crypted).startswith("$6$"):
        raise dbus.exceptions.DBusException("not a crypted password", name="org.freedesktop.Accounts.Error.Failed")
    self.Set(USER_IFACE, "MockPasswordHash", crypted)
    self.Set(USER_IFACE, "PasswordHint", hint)
    self.Set(USER_IFACE, "PasswordMode", dbus.Int32(0))
    self.EmitSignal(USER_IFACE, "Changed", "", [])


def _set_automatic_login(self, enabled):
    _require_authorization()
    if enabled:
        for path in _users():
            other = mockobject.objects[path]
            if other is not self and other.props[USER_IFACE]["AutomaticLogin"]:
                other.Set(USER_IFACE, "AutomaticLogin", False)
    self.Set(USER_IFACE, "AutomaticLogin", bool(enabled))
    users = [dbus.ObjectPath(path) for path in _users() if mockobject.objects[path].props[USER_IFACE]["AutomaticLogin"]]
    _main().Set(MAIN_IFACE, "AutomaticLoginUsers", dbus.Array(users, signature="o"))
    self.EmitSignal(USER_IFACE, "Changed", "", [])


def add_user(main, uid, name, real_name, admin):
    path = _user_path(uid)
    main.AddObject(
        path,
        USER_IFACE,
        {
            "Uid": dbus.UInt64(uid),
            "UserName": name,
            "RealName": real_name,
            "AccountType": dbus.Int32(1 if admin else 0),
            "HomeDirectory": f"/home/{name}",
            "Shell": "/bin/bash",
            "Email": "",
            "Language": "",
            "IconFile": "",
            "Locked": False,
            "PasswordMode": dbus.Int32(0),
            "PasswordHint": "",
            "AutomaticLogin": False,
            "SystemAccount": False,
            "LocalAccount": True,
            "MockPasswordHash": "",
        },
        [
            ("SetRealName", "s", "", _setter("RealName")),
            ("SetIconFile", "s", "", _setter("IconFile")),
            ("SetPasswordHint", "s", "", _setter("PasswordHint")),
            ("SetAccountType", "i", "", _setter("AccountType")),
            ("SetAutomaticLogin", "b", "", _set_automatic_login),
            ("SetPassword", "ss", "", _set_password),
        ],
    )
    main.EmitSignal(MAIN_IFACE, "UserAdded", "o", [dbus.ObjectPath(path)])
    return path


def _list_cached_users(self):
    return dbus.Array([dbus.ObjectPath(path) for path in _users()], signature="o")


def _find_user_by_id(self, uid):
    path = _user_path(uid)
    if path not in mockobject.objects:
        raise dbus.exceptions.DBusException(
            f"no user {uid}", name="org.freedesktop.Accounts.Error.Failed"
        )
    return dbus.ObjectPath(path)


def _find_user_by_name(self, name):
    for path in _users():
        if mockobject.objects[path].props[USER_IFACE]["UserName"] == name:
            return dbus.ObjectPath(path)
    raise dbus.exceptions.DBusException(f"no user {name}", name="org.freedesktop.Accounts.Error.Failed")


def _create_user(self, name, real_name, account_type):
    _require_authorization()
    for path in _users():
        if mockobject.objects[path].props[USER_IFACE]["UserName"] == name:
            raise dbus.exceptions.DBusException(
                f"A user with name '{name}' already exists", name="org.freedesktop.Accounts.Error.UserExists"
            )
    uid = 3000
    while _user_path(uid) in mockobject.objects:
        uid += 1
    return dbus.ObjectPath(add_user(self, uid, name, real_name, int(account_type) == 1))


def _delete_user(self, uid, remove_files):
    _require_authorization()
    path = _user_path(uid)
    if path not in mockobject.objects:
        raise dbus.exceptions.DBusException(
            f"No user with uid {uid} found", name="org.freedesktop.Accounts.Error.UserDoesNotExist"
        )
    self.RemoveObject(path)
    STATE["deleted"].append((int(uid), bool(remove_files)))
    self.EmitSignal(MAIN_IFACE, "UserDeleted", "o", [dbus.ObjectPath(path)])


def load(mock, parameters):
    STATE["authorized"] = True
    STATE["deleted"] = []
    mock.AddMethods(
        MAIN_IFACE,
        [
            ("ListCachedUsers", "", "ao", _list_cached_users),
            ("FindUserById", "x", "o", _find_user_by_id),
            ("FindUserByName", "s", "o", _find_user_by_name),
            ("CreateUser", "ssi", "o", _create_user),
            ("DeleteUser", "xb", "", _delete_user),
        ],
    )
    mock.AddProperties(
        MAIN_IFACE,
        dbus.Dictionary(
            {
                "DaemonVersion": "23.13",
                "HasNoUsers": False,
                "HasMultipleUsers": True,
                "AutomaticLoginUsers": dbus.Array([], signature="o"),
            },
            signature="sv",
        ),
    )
    for user in parameters.get("users", DEFAULT_USERS):
        add_user(mock, int(user["uid"]), user["name"], user.get("real_name", ""), bool(user.get("admin", False)))


@dbus.service.method(MOCK_IFACE, in_signature="b", out_signature="")
def SetAuthorized(self, authorized):
    """Make every mutation succeed (True) or fail as polkit refusing (False)."""
    STATE["authorized"] = bool(authorized)


@dbus.service.method(MOCK_IFACE, in_signature="", out_signature="a(xb)")
def GetDeletedUsers(self):
    """(uid, removeFiles) for every DeleteUser call that succeeded."""
    return dbus.Array([(dbus.Int64(uid), remove) for uid, remove in STATE["deleted"]], signature="(xb)")
