//! Policy boundary for future XDG Wallpaper portal backend requests.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetOn {
    Background,
    LockScreen,
    Both,
}

#[derive(Clone, Eq, PartialEq)]
pub struct Request {
    pub app_id: String,
    pub uri: String,
    pub set_on: SetOn,
    pub show_preview: bool,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Request")
            .field("app_id", &self.app_id)
            .field("uri", &"<private>")
            .field("set_on", &self.set_on)
            .field("show_preview", &self.show_preview)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    InvalidAppId,
    InvalidUri,
    RemoteUriUnsupported,
    SecureLockUnavailable,
    UnsafeSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Plan {
    pub app_id: String,
    pub source: super::Source,
    /// The rmac backend always confirms a desktop-wide mutation, even if the
    /// caller did not request a preview.
    pub preview_required: bool,
    /// Portal v1 has no per-output model. Consent therefore means replacing
    /// the whole-desktop default and its overrides as one visible transaction.
    pub clear_per_output_overrides: bool,
    pub fit: rmac_shell_settings::WallpaperFit,
}

pub fn evaluate(request: Request) -> Result<Plan, ErrorKind> {
    if request.app_id.trim().is_empty()
        || request.app_id.len() > 256
        || request.app_id.chars().any(char::is_control)
    {
        return Err(ErrorKind::InvalidAppId);
    }
    if request.set_on != SetOn::Background {
        return Err(ErrorKind::SecureLockUnavailable);
    }
    let url = url::Url::parse(&request.uri).map_err(|_| ErrorKind::InvalidUri)?;
    if url.scheme() != "file" {
        return Err(ErrorKind::RemoteUriUnsupported);
    }
    if url.host_str().is_some() {
        return Err(ErrorKind::UnsafeSource);
    }
    let source = super::parse_source(Some(&request.uri)).map_err(|_| ErrorKind::UnsafeSource)?;
    Ok(Plan {
        app_id: request.app_id,
        source,
        preview_required: true,
        clear_per_output_overrides: true,
        fit: rmac_shell_settings::WallpaperFit::Fill,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(uri: &str) -> Request {
        Request {
            app_id: "org.example.Photos".into(),
            uri: uri.into(),
            set_on: SetOn::Background,
            show_preview: false,
        }
    }

    #[test]
    fn local_background_request_becomes_a_confirmed_whole_desktop_transaction() {
        let plan = evaluate(request("file:///run/user/1000/doc/photo.png")).unwrap();
        assert!(plan.preview_required);
        assert!(plan.clear_per_output_overrides);
        assert_eq!(plan.fit, rmac_shell_settings::WallpaperFit::Fill);
        assert!(matches!(plan.source, super::super::Source::File(_)));
    }

    #[test]
    fn remote_and_lock_screen_requests_fail_explicitly() {
        assert_eq!(
            evaluate(request("https://example.com/photo.png")).unwrap_err(),
            ErrorKind::RemoteUriUnsupported
        );
        let mut lock = request("file:///tmp/photo.png");
        lock.set_on = SetOn::Both;
        assert_eq!(
            evaluate(lock).unwrap_err(),
            ErrorKind::SecureLockUnavailable
        );
    }

    #[test]
    fn malformed_identity_and_hosted_file_uri_never_reach_preview() {
        let mut invalid = request("file:///tmp/photo.png");
        invalid.app_id = "\n".into();
        assert_eq!(evaluate(invalid).unwrap_err(), ErrorKind::InvalidAppId);
        assert_eq!(
            evaluate(request("file://server/share/photo.png")).unwrap_err(),
            ErrorKind::UnsafeSource
        );
        assert!(!format!("{:?}", request("file:///home/alex/private.png")).contains("alex"));
    }
}
