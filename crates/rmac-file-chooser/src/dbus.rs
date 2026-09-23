//! `org.freedesktop.impl.portal.FileChooser` on the session bus.
//!
//! Only the current owner of `org.freedesktop.portal.Desktop` may call the
//! backend; the frontend is what turns the returned URIs into document-portal
//! grants for sandboxed callers.

use zbus::connection::Builder;
use zbus::fdo;
use zbus::message::Header;
use zbus::zvariant::{DeserializeDict, OwnedObjectPath, SerializeDict, Type};
use zbus::{interface, Connection};

use crate::outcome::{results, Outcome, Response};
use crate::request::{Filter, Mode, RawOptions, Request, Rule, WireChoice, WireFilter};
use crate::service::{Broker, Close};

pub const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.rmac.filechooser";
pub const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";

#[derive(Debug, Default, DeserializeDict, Type)]
#[zvariant(signature = "dict")]
pub struct WireOptions {
    accept_label: Option<String>,
    modal: Option<bool>,
    multiple: Option<bool>,
    directory: Option<bool>,
    filters: Option<Vec<WireFilter>>,
    current_filter: Option<WireFilter>,
    choices: Option<Vec<WireChoice>>,
    current_name: Option<String>,
    current_folder: Option<Vec<u8>>,
    current_file: Option<Vec<u8>>,
    files: Option<Vec<Vec<u8>>>,
}

impl From<WireOptions> for RawOptions {
    fn from(wire: WireOptions) -> Self {
        Self {
            accept_label: wire.accept_label,
            modal: wire.modal,
            multiple: wire.multiple,
            directory: wire.directory,
            filters: wire.filters,
            current_filter: wire.current_filter,
            choices: wire.choices,
            current_name: wire.current_name,
            current_folder: wire.current_folder,
            current_file: wire.current_file,
            files: wire.files,
        }
    }
}

#[derive(Debug, Default, SerializeDict, Type)]
#[zvariant(signature = "dict")]
pub struct WireResults {
    uris: Option<Vec<String>>,
    choices: Option<Vec<(String, String)>>,
    current_filter: Option<WireFilter>,
}

fn wire_filter(filter: &Filter) -> WireFilter {
    (
        filter.name.clone(),
        filter
            .rules
            .iter()
            .map(|rule| match rule {
                Rule::Glob(glob) => (0, glob.clone()),
                Rule::Mime(mime) => (1, mime.clone()),
            })
            .collect(),
    )
}

#[derive(Clone, Debug)]
pub struct FileChooserInterface {
    broker: Broker,
}

impl FileChooserInterface {
    // The backend protocol fixes the argument list; zbus injects the header
    // and connection needed to authenticate the caller.
    #[allow(clippy::too_many_arguments)]
    async fn present(
        &self,
        mode: Mode,
        handle: OwnedObjectPath,
        app_id: String,
        parent_window: String,
        title: String,
        options: WireOptions,
        header: &Header<'_>,
        connection: &Connection,
    ) -> fdo::Result<(u32, WireResults)> {
        verify_portal_caller(connection, header).await?;
        let request = Request::from_wire(mode, app_id, &parent_window, title, options.into())
            .map_err(|error| fdo::Error::InvalidArgs(error.to_string()))?;
        let filters = request.filters.clone();
        let (close, panel_closed, adapter_closed) = Broker::close_pair();
        let exported = connection
            .object_server()
            .at(
                handle.clone(),
                RequestInterface {
                    close: close.clone(),
                },
            )
            .await
            .map_err(fdo::Error::ZBus)?;
        if !exported {
            return Err(fdo::Error::InvalidArgs(
                "FileChooser request handle is already in use".into(),
            ));
        }
        let outcome = self
            .broker
            .present(request, panel_closed, adapter_closed)
            .await;
        let _ = connection
            .object_server()
            .remove::<RequestInterface, _>(handle)
            .await;
        drop(close);
        Ok(match outcome {
            Ok(Outcome::Chosen(selection)) => match results(&selection) {
                Some(chosen) => (
                    Response::Success as u32,
                    WireResults {
                        uris: Some(chosen.uris),
                        choices: (!chosen.choices.is_empty()).then_some(chosen.choices),
                        current_filter: chosen
                            .current_filter
                            .and_then(|index| filters.get(index))
                            .map(wire_filter),
                    },
                ),
                None => (Response::Other as u32, WireResults::default()),
            },
            Ok(Outcome::Cancelled) => (Response::Cancelled as u32, WireResults::default()),
            Err(_) => (Response::Other as u32, WireResults::default()),
        })
    }
}

#[interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FileChooserInterface {
    #[allow(clippy::too_many_arguments)]
    #[zbus(name = "OpenFile")]
    async fn open_file(
        &self,
        handle: OwnedObjectPath,
        app_id: String,
        parent_window: String,
        title: String,
        options: WireOptions,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<(u32, WireResults)> {
        self.present(
            Mode::Open,
            handle,
            app_id,
            parent_window,
            title,
            options,
            &header,
            connection,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    #[zbus(name = "SaveFile")]
    async fn save_file(
        &self,
        handle: OwnedObjectPath,
        app_id: String,
        parent_window: String,
        title: String,
        options: WireOptions,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<(u32, WireResults)> {
        self.present(
            Mode::Save,
            handle,
            app_id,
            parent_window,
            title,
            options,
            &header,
            connection,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    #[zbus(name = "SaveFiles")]
    async fn save_files(
        &self,
        handle: OwnedObjectPath,
        app_id: String,
        parent_window: String,
        title: String,
        options: WireOptions,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<(u32, WireResults)> {
        self.present(
            Mode::SaveFiles,
            handle,
            app_id,
            parent_window,
            title,
            options,
            &header,
            connection,
        )
        .await
    }
}

#[derive(Clone, Debug)]
pub struct RequestInterface {
    close: Close,
}

#[interface(name = "org.freedesktop.impl.portal.Request")]
impl RequestInterface {
    fn close(&self) {
        self.close.close();
    }
}

/// Own the backend name and export the interface. Keep the returned
/// connection alive for the life of the process.
pub async fn serve(broker: Broker) -> zbus::Result<Connection> {
    Builder::session()?
        .name(BUS_NAME)?
        .serve_at(PORTAL_PATH, FileChooserInterface { broker })?
        .build()
        .await
}

async fn verify_portal_caller(connection: &Connection, header: &Header<'_>) -> fdo::Result<()> {
    let sender = header
        .sender()
        .ok_or_else(|| fdo::Error::AccessDenied("FileChooser sender is unavailable".into()))?;
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
            "FileChooser backend calls require xdg-desktop-portal".into(),
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

    #[test]
    fn only_the_exact_frontend_owner_is_authenticated() {
        assert!(portal_owner_matches(":1.42", ":1.42"));
        assert!(!portal_owner_matches(":1.43", ":1.42"));
        assert!(!portal_owner_matches("", ""));
    }

    #[test]
    fn introspection_matches_the_backend_contract() {
        let (broker, _panels) = Broker::channel();
        let chooser = FileChooserInterface { broker };
        let mut xml = String::new();
        chooser.introspect_to_writer(&mut xml, 0);
        assert!(xml.contains("org.freedesktop.impl.portal.FileChooser"));
        for method in ["OpenFile", "SaveFile", "SaveFiles"] {
            assert!(xml.contains(&format!("method name=\"{method}\"")));
        }
        assert!(xml.contains("type=\"a{sv}\""));

        let (close, panel_closed, adapter_closed) = Broker::close_pair();
        let request = RequestInterface { close };
        let mut request_xml = String::new();
        request.introspect_to_writer(&mut request_xml, 0);
        assert!(request_xml.contains("method name=\"Close\""));
        request.close();
        assert!(panel_closed.try_recv().is_ok());
        assert!(adapter_closed.try_recv().is_ok());
    }

    #[test]
    fn filters_round_trip_to_the_wire_shape() {
        let filter = Filter {
            name: "Images".into(),
            rules: vec![Rule::Mime("image/png".into()), Rule::Glob("*.jpg".into())],
        };
        assert_eq!(
            wire_filter(&filter),
            (
                "Images".to_owned(),
                vec![(1, "image/png".to_owned()), (0, "*.jpg".to_owned())]
            )
        );
    }
}
