//! Printer administration through cups-pk-helper
//! (`org.opensuse.CupsPkHelper.Mechanism` on the system bus).
//!
//! cups-pk-helper is CUPS's polkit front end: each method checks its own
//! polkit action (`printeraddremove`, `printer-enable`, `job-edit`,
//! `devices-get`) for the calling process and only then talks to cupsd as
//! an administrator. Ubuntu's polkit rules allow members of `sudo` and
//! `lpadmin` in an active local session without a password; everyone else
//! gets polkit's own authentication (or a refusal when no agent runs).
//! Lulo never runs lpadmin, sudo or anything as root itself.

use std::collections::HashMap;

use zbus::blocking::{Connection, Proxy};
use zbus::proxy::MethodFlags;

use crate::model::{devices_from_flat, validate_printer_name, Device};
use crate::Error;

const SERVICE: &str = "org.opensuse.CupsPkHelper.Mechanism";
const PATH: &str = "/";
const INTERFACE: &str = "org.opensuse.CupsPkHelper.Mechanism";
/// cupsd's model name for a queue it builds from the printer's own IPP
/// description (IPP Everywhere), with no vendor driver.
pub const DRIVERLESS_MODEL: &str = "everywhere";

pub struct PrinterAdmin {
    connection: Connection,
}

impl PrinterAdmin {
    pub fn system() -> Result<Self, Error> {
        Ok(Self {
            connection: Connection::system().map_err(|_| Error::Unavailable)?,
        })
    }

    pub fn on_connection(connection: Connection) -> Self {
        Self { connection }
    }

    fn call<B, R>(&self, method: &str, body: &B) -> Result<R, Error>
    where
        B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType,
        R: for<'d> zbus::zvariant::DynamicDeserialize<'d>,
    {
        let proxy = Proxy::new(&self.connection, SERVICE, PATH, INTERFACE)
            .map_err(|_| Error::Unavailable)?;
        proxy
            .call_with_flags(method, MethodFlags::AllowInteractiveAuth.into(), body)
            .map_err(map_error)?
            .ok_or(Error::Failed)
    }

    /// cups-pk-helper reports CUPS failures as a non-empty string.
    fn status(
        &self,
        method: &str,
        body: &(impl zbus::export::serde::Serialize + zbus::zvariant::DynamicType),
    ) -> Result<(), Error> {
        let error: String = self.call(method, body)?;
        if error.is_empty() {
            Ok(())
        } else if error.to_ascii_lowercase().contains("not authorized")
            || error.to_ascii_lowercase().contains("forbidden")
        {
            Err(Error::NotAuthorized)
        } else {
            Err(Error::Failed)
        }
    }

    /// Printers CUPS can see on the network and local ports that it can set
    /// up without a driver. Blocks for up to `timeout_seconds` while CUPS's
    /// backends browse.
    pub fn discover(&self, timeout_seconds: i32) -> Result<Vec<Device>, Error> {
        let include: Vec<&str> = Vec::new();
        let exclude: Vec<&str> = Vec::new();
        let (error, devices): (String, HashMap<String, String>) = self.call(
            "DevicesGet",
            &(timeout_seconds.clamp(1, 60), 0_i32, include, exclude),
        )?;
        if !error.is_empty() && devices.is_empty() {
            return Err(Error::Failed);
        }
        Ok(devices_from_flat(&devices))
    }

    /// Add a driverless queue for `device` named `name`, then enable it and
    /// let it accept jobs.
    pub fn add_printer(&self, name: &str, device: &Device, location: &str) -> Result<(), Error> {
        if !validate_printer_name(name) || !device.is_driverless() {
            return Err(Error::Failed);
        }
        let info = device.display_name().chars().take(127).collect::<String>();
        let location = location
            .chars()
            .filter(|c| !c.is_control())
            .take(127)
            .collect::<String>();
        self.status(
            "PrinterAdd",
            &(
                name,
                device.uri.as_str(),
                DRIVERLESS_MODEL,
                info.as_str(),
                location.as_str(),
            ),
        )?;
        // Best effort: a new queue normally starts enabled and accepting.
        let _ = self.status("PrinterSetEnabled", &(name, true));
        let _ = self.status("PrinterSetAcceptJobs", &(name, true, ""));
        Ok(())
    }

    pub fn delete_printer(&self, name: &str) -> Result<(), Error> {
        if !validate_printer_name(name) {
            return Err(Error::Failed);
        }
        self.status("PrinterDelete", &(name,))
    }

    /// Pause (false) or resume (true) a printer.
    pub fn set_enabled(&self, name: &str, enabled: bool) -> Result<(), Error> {
        if !validate_printer_name(name) {
            return Err(Error::Failed);
        }
        self.status("PrinterSetEnabled", &(name, enabled))
    }

    /// Cancel one job (without purging its history).
    pub fn cancel_job(&self, job_id: i32) -> Result<(), Error> {
        if job_id <= 0 {
            return Err(Error::Failed);
        }
        self.status("JobCancelPurge", &(job_id, false))
    }
}

fn map_error(error: zbus::Error) -> Error {
    match &error {
        zbus::Error::MethodError(name, _, _) => match name.as_str() {
            "org.opensuse.CupsPkHelper.Mechanism.NotPrivileged"
            | "org.freedesktop.DBus.Error.AccessDenied"
            | "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired" => Error::NotAuthorized,
            "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.NameHasNoOwner"
            | "org.freedesktop.DBus.Error.NoReply" => Error::Unavailable,
            _ => Error::Failed,
        },
        zbus::Error::InputOutput(_) | zbus::Error::Address(_) => Error::Unavailable,
        _ => Error::Failed,
    }
}
