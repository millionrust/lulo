//! Authenticated XDG Wallpaper backend wire adapter.

use std::collections::HashMap;
use std::fmt;

use zbus::connection::Builder;
use zbus::fdo;
use zbus::message::Header;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};
use zbus::{interface, Connection};

use crate::broker::{Broker, Cancellation, PreviewEvent};
use crate::{Consent, Importer, PortalResponse, RequestId};

pub const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.rmac.wallpaper";
pub const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireErrorKind {
    WrongType,
    InvalidValue,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WireError {
    pub field: &'static str,
    pub kind: WireErrorKind,
}

impl fmt::Display for WireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid Wallpaper portal option {} ({:?})",
            self.field, self.kind
        )
    }
}

impl std::error::Error for WireError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Options {
    show_preview: bool,
    set_on: rmac_wallpaper::portal::SetOn,
}

#[derive(Clone, Debug)]
pub struct WallpaperInterface {
    broker: Broker,
}

impl WallpaperInterface {
    pub fn new(broker: Broker) -> Self {
        Self { broker }
    }
}

#[interface(name = "org.freedesktop.impl.portal.Wallpaper")]
impl WallpaperInterface {
    // The backend protocol fixes five input arguments and zbus injects the
    // authenticated header plus connection needed to verify the caller.
    #[allow(clippy::too_many_arguments)]
    #[zbus(name = "SetWallpaperURI")]
    async fn set_wallpaper_uri(
        &self,
        handle: OwnedObjectPath,
        app_id: String,
        parent_window: String,
        uri: String,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<u32> {
        verify_portal_caller(connection, &header).await?;
        let options = decode_options(options).map_err(|error| {
            fdo::Error::InvalidArgs(format!("Wallpaper portal options are invalid: {error}"))
        })?;
        let cancellation = Cancellation::new();
        let exported = connection
            .object_server()
            .at(
                handle.clone(),
                RequestInterface {
                    cancellation: cancellation.clone(),
                },
            )
            .await
            .map_err(fdo::Error::ZBus)?;
        if !exported {
            return Err(fdo::Error::InvalidArgs(
                "Wallpaper request handle is already in use".into(),
            ));
        }
        let response = self
            .broker
            .request(
                rmac_wallpaper::portal::Request {
                    app_id,
                    uri,
                    set_on: options.set_on,
                    show_preview: options.show_preview,
                },
                parent_window,
                cancellation,
            )
            .await;
        let removed = connection
            .object_server()
            .remove::<RequestInterface, _>(handle)
            .await;
        match removed {
            Ok(true) => Ok(response as u32),
            Ok(false) | Err(_) => Ok(PortalResponse::Other as u32),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RequestInterface {
    cancellation: Cancellation,
}

#[interface(name = "org.freedesktop.impl.portal.Request")]
impl RequestInterface {
    fn close(&self) {
        self.cancellation.cancel();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceError {
    Bus,
}

impl fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Wallpaper portal service is unavailable")
    }
}

impl std::error::Error for ServiceError {}

#[derive(Clone)]
pub struct ServiceHandle {
    _connection: Connection,
    broker: Broker,
}

impl ServiceHandle {
    pub fn decide(&self, id: RequestId, consent: Consent) -> bool {
        self.broker.decide(id, consent)
    }

    pub fn pending_count(&self) -> usize {
        self.broker.pending_count()
    }
}

impl fmt::Debug for ServiceHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServiceHandle")
            .field("pending", &self.pending_count())
            .finish_non_exhaustive()
    }
}

/// Own the dedicated backend name and return the private preview stream which
/// the supervised UI process must drain for the service to become useful.
pub async fn serve(
    importer: Importer,
) -> Result<(ServiceHandle, async_channel::Receiver<PreviewEvent>), ServiceError> {
    let (broker, previews) = Broker::new(importer);
    let connection = Builder::session()
        .map_err(|_| ServiceError::Bus)?
        .name(BUS_NAME)
        .map_err(|_| ServiceError::Bus)?
        .serve_at(PORTAL_PATH, WallpaperInterface::new(broker.clone()))
        .map_err(|_| ServiceError::Bus)?
        .build()
        .await
        .map_err(|_| ServiceError::Bus)?;
    Ok((
        ServiceHandle {
            _connection: connection,
            broker,
        },
        previews,
    ))
}

fn decode_options(mut values: HashMap<String, OwnedValue>) -> Result<Options, WireError> {
    let show_preview = take::<bool>(&mut values, "show-preview")?.unwrap_or(false);
    let set_on = match take::<String>(&mut values, "set-on")?.as_deref() {
        None | Some("background") => rmac_wallpaper::portal::SetOn::Background,
        Some("lockscreen") => rmac_wallpaper::portal::SetOn::LockScreen,
        Some("both") => rmac_wallpaper::portal::SetOn::Both,
        Some(_) => {
            return Err(WireError {
                field: "set-on",
                kind: WireErrorKind::InvalidValue,
            });
        }
    };
    Ok(Options {
        show_preview,
        set_on,
    })
}

fn take<T>(
    values: &mut HashMap<String, OwnedValue>,
    field: &'static str,
) -> Result<Option<T>, WireError>
where
    T: TryFrom<OwnedValue>,
{
    let Some(value) = values.remove(field) else {
        return Ok(None);
    };
    T::try_from(value).map(Some).map_err(|_| WireError {
        field,
        kind: WireErrorKind::WrongType,
    })
}

async fn verify_portal_caller(connection: &Connection, header: &Header<'_>) -> fdo::Result<()> {
    let sender = header
        .sender()
        .ok_or_else(|| fdo::Error::AccessDenied("Wallpaper portal sender is unavailable".into()))?;
    let proxy = fdo::DBusProxy::new(connection)
        .await
        .map_err(fdo::Error::ZBus)?;
    let name = "org.freedesktop.portal.Desktop"
        .try_into()
        .map_err(|_| fdo::Error::Failed("portal service name is invalid".into()))?;
    let owner = proxy.get_name_owner(name).await?;
    if portal_owner_matches(sender.as_str(), owner.as_str()) {
        Ok(())
    } else {
        Err(fdo::Error::AccessDenied(
            "Wallpaper backend calls require xdg-desktop-portal".into(),
        ))
    }
}

fn portal_owner_matches(sender: &str, owner: &str) -> bool {
    !sender.is_empty() && sender == owner
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::object_server::Interface as _;
    use zbus::zvariant::Str;

    #[test]
    fn options_are_typed_defaulted_and_forward_compatible() {
        let defaults = decode_options(HashMap::new()).unwrap();
        assert!(!defaults.show_preview);
        assert_eq!(defaults.set_on, rmac_wallpaper::portal::SetOn::Background);

        let options = decode_options(HashMap::from([
            ("show-preview".into(), OwnedValue::from(true)),
            (
                "set-on".into(),
                OwnedValue::from(Str::from("lockscreen".to_owned())),
            ),
            ("future-option".into(), OwnedValue::from(42_u32)),
        ]))
        .unwrap();
        assert!(options.show_preview);
        assert_eq!(options.set_on, rmac_wallpaper::portal::SetOn::LockScreen);

        let wrong_type = decode_options(HashMap::from([(
            "show-preview".into(),
            OwnedValue::from(1_u32),
        )]))
        .unwrap_err();
        assert_eq!(wrong_type.field, "show-preview");
        assert_eq!(wrong_type.kind, WireErrorKind::WrongType);

        let invalid = decode_options(HashMap::from([(
            "set-on".into(),
            OwnedValue::from(Str::from("desktop".to_owned())),
        )]))
        .unwrap_err();
        assert_eq!(invalid.kind, WireErrorKind::InvalidValue);
    }

    #[test]
    fn only_the_exact_frontend_owner_is_authenticated() {
        assert!(portal_owner_matches(":1.42", ":1.42"));
        assert!(!portal_owner_matches(":1.43", ":1.42"));
        assert!(!portal_owner_matches("", ""));
    }

    #[test]
    fn introspection_matches_the_backend_contract() {
        let cancellation = Cancellation::new();
        let request = RequestInterface {
            cancellation: cancellation.clone(),
        };
        let mut request_xml = String::new();
        request.introspect_to_writer(&mut request_xml, 0);
        assert!(request_xml.contains("org.freedesktop.impl.portal.Request"));
        assert!(request_xml.contains("method name=\"Close\""));
        assert!(!request_xml.contains("property name="));
        request.close();
        assert!(cancellation.is_cancelled());
    }
}
