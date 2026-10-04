//! A private D-Bus daemon with one python-dbusmock template on it, for
//! Rust integration tests (`#[path]`-included by the crates that use it).
//! The real system and session buses are never touched: the daemon is
//! started here and the mock is pointed at it explicitly.

#![allow(dead_code)]

use std::io::{BufRead as _, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const PYTHON: &str = "/usr/bin/python3";
const MOCK_INTERFACE: &str = "org.freedesktop.DBus.Mock";

pub struct MockBus {
    daemon: Child,
    mock: Child,
    address: String,
    main_object: String,
    bus_name: String,
}

fn required() -> bool {
    std::env::var_os("RMAC_REQUIRE_DBUSMOCK").is_some_and(|value| value == "1")
}

fn skip(reason: &str) -> Option<MockBus> {
    assert!(
        !required(),
        "python-dbusmock test bus unavailable: {reason}"
    );
    eprintln!("skipping: {reason}");
    None
}

fn template(file: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/dbusmock")
        .join(file)
}

impl MockBus {
    /// Start a private bus and `file`'s template on it, then wait until
    /// `bus_name` is owned. `None` (test skipped) when the tools are missing
    /// and RMAC_REQUIRE_DBUSMOCK is not set.
    pub fn start(file: &str, bus_name: &str, parameters: &str) -> Option<Self> {
        let has_mock = Command::new(PYTHON)
            .args(["-c", "import dbusmock"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !has_mock {
            return skip("python3-dbusmock is not installed");
        }
        let mut daemon = match Command::new("dbus-daemon") // wording: internal
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(daemon) => daemon,
            Err(_) => return skip("the private bus daemon is not installed"),
        };
        let mut line = String::new();
        let stdout = daemon.stdout.take().expect("daemon stdout");
        BufReader::new(stdout)
            .read_line(&mut line)
            .expect("bus address");
        let address = line.trim().to_owned();
        assert!(!address.is_empty(), "the private bus printed no address");
        let mock = Command::new(PYTHON)
            .args(["-m", "dbusmock", "--system", "--template"])
            .arg(template(file))
            .args(["--parameters", parameters])
            .env("DBUS_SYSTEM_BUS_ADDRESS", &address)
            .env("DBUS_SESSION_BUS_ADDRESS", &address)
            .stdout(Stdio::null())
            .spawn()
            .expect("start python-dbusmock");
        let module = std::fs::read_to_string(template(file)).expect("template");
        let main_object = module
            .lines()
            .find_map(|line| line.strip_prefix("MAIN_OBJ = "))
            .map(|value| value.trim().trim_matches('"').to_owned())
            .expect("MAIN_OBJ in template");
        let mut bus = Self {
            daemon,
            mock,
            address,
            main_object,
            bus_name: bus_name.to_owned(),
        };
        bus.wait_for(bus_name);
        Some(bus)
    }

    fn wait_for(&mut self, bus_name: &str) {
        let connection = self.connection();
        let proxy = zbus::blocking::fdo::DBusProxy::new(&connection).expect("bus proxy");
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let name = zbus::names::BusName::try_from(bus_name).expect("bus name");
            if proxy.name_has_owner(name).unwrap_or(false) {
                return;
            }
            if let Ok(Some(status)) = self.mock.try_wait() {
                panic!("python-dbusmock exited early: {status}");
            }
            assert!(
                Instant::now() < deadline,
                "{bus_name} never appeared on the private bus"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    /// A new connection to the private bus.
    pub fn connection(&self) -> zbus::blocking::Connection {
        zbus::blocking::connection::Builder::address(self.address.as_str())
            .expect("bus address")
            .build()
            .expect("connect to the private bus")
    }

    /// Read `interface.name` on `path` straight from the mock.
    pub fn property<T>(&self, path: &str, interface: &str, name: &str) -> T
    where
        T: TryFrom<zbus::zvariant::OwnedValue>,
        T::Error: std::fmt::Debug,
    {
        let connection = self.connection();
        let destination = self.destination();
        let proxy = zbus::blocking::Proxy::new(
            &connection,
            destination.as_str(),
            path,
            "org.freedesktop.DBus.Properties",
        )
        .expect("properties proxy");
        let value: zbus::zvariant::OwnedValue = proxy.call("Get", &(interface, name)).expect("Get");
        T::try_from(value).expect("property type")
    }

    pub fn user_property<T>(&self, path: &str, name: &str) -> T
    where
        T: TryFrom<zbus::zvariant::OwnedValue>,
        T::Error: std::fmt::Debug,
    {
        self.property(path, "org.freedesktop.Accounts.User", name)
    }

    fn destination(&self) -> String {
        self.bus_name.clone()
    }

    /// Call a method on the mock control interface of the main object.
    pub fn call_mock<R>(&self, method: &str) -> R
    where
        R: for<'d> zbus::zvariant::DynamicDeserialize<'d>,
    {
        self.call_mock_with(method, &())
    }

    pub fn call_mock_with<B, R>(&self, method: &str, body: &B) -> R
    where
        B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType,
        R: for<'d> zbus::zvariant::DynamicDeserialize<'d>,
    {
        let connection = self.connection();
        let destination = self.destination();
        let proxy = zbus::blocking::Proxy::new(
            &connection,
            destination.as_str(),
            self.main_object.as_str(),
            MOCK_INTERFACE,
        )
        .expect("mock proxy");
        proxy.call(method, body).expect("mock call")
    }
}

impl Drop for MockBus {
    fn drop(&mut self) {
        let _ = self.mock.kill();
        let _ = self.mock.wait();
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}
