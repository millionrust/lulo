use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use crate::{BackendError, FailureKind};

pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait Backend: Send + Sync + 'static {
    fn launch<'a>(
        &'a self,
        spec: &'a rmac_apps::LaunchSpec,
    ) -> BackendFuture<'a, Result<rmac_app_launch::Outcome, BackendError>>;

    fn open_setting<'a>(&'a self, pane_id: &'a str) -> BackendFuture<'a, Result<(), BackendError>>;

    fn open_file<'a>(&'a self, path: &'a Path) -> BackendFuture<'a, Result<(), BackendError>>;

    fn reveal_file<'a>(&'a self, path: &'a Path) -> BackendFuture<'a, Result<(), BackendError>>;

    fn copy_text<'a>(&'a self, text: &'a str) -> BackendFuture<'a, Result<(), BackendError>>;

    /// Open Files searching for `query` ("Search in Files").
    fn search_files<'a>(&'a self, query: &'a str) -> BackendFuture<'a, Result<(), BackendError>> {
        let _ = query;
        Box::pin(async {
            Err(BackendError::new(
                FailureKind::Unavailable,
                "Files search is unavailable",
            ))
        })
    }
}

/// UI-owned operations that require the live launcher application context.
pub trait Surface: Send + Sync + 'static {
    fn open_setting(&self, pane_id: &str) -> Result<(), BackendError>;
    fn copy_text(&self, text: &str) -> Result<(), BackendError>;

    fn search_files(&self, query: &str) -> Result<(), BackendError> {
        let _ = query;
        Err(BackendError::new(
            FailureKind::Unavailable,
            "Files search is unavailable",
        ))
    }
}

#[derive(Clone, Debug)]
pub struct SystemBackend<S> {
    surface: S,
}

impl<S> SystemBackend<S> {
    pub fn new(surface: S) -> Self {
        Self { surface }
    }
}

impl<S: Surface> Backend for SystemBackend<S> {
    fn launch<'a>(
        &'a self,
        spec: &'a rmac_apps::LaunchSpec,
    ) -> BackendFuture<'a, Result<rmac_app_launch::Outcome, BackendError>> {
        let spec = spec.clone();
        Box::pin(async move {
            rmac_app_launch::launch(spec).await.map_err(|error| {
                BackendError::new(
                    match error.kind {
                        rmac_app_launch::ErrorKind::Io(kind) => FailureKind::Io(kind),
                        rmac_app_launch::ErrorKind::Rejected => FailureKind::Rejected,
                        rmac_app_launch::ErrorKind::Protocol => FailureKind::Other,
                    },
                    error.to_string(),
                )
            })
        })
    }

    fn open_setting<'a>(&'a self, pane_id: &'a str) -> BackendFuture<'a, Result<(), BackendError>> {
        Box::pin(async move { self.surface.open_setting(pane_id) })
    }

    fn open_file<'a>(&'a self, path: &'a Path) -> BackendFuture<'a, Result<(), BackendError>> {
        let path = path.to_path_buf();
        Box::pin(async move { rmac_app_launch::open_item(path).await.map_err(item_error) })
    }

    fn reveal_file<'a>(&'a self, path: &'a Path) -> BackendFuture<'a, Result<(), BackendError>> {
        let path = path.to_path_buf();
        Box::pin(async move { rmac_app_launch::reveal_item(path).await.map_err(item_error) })
    }

    fn copy_text<'a>(&'a self, text: &'a str) -> BackendFuture<'a, Result<(), BackendError>> {
        Box::pin(async move { self.surface.copy_text(text) })
    }

    fn search_files<'a>(&'a self, query: &'a str) -> BackendFuture<'a, Result<(), BackendError>> {
        Box::pin(async move { self.surface.search_files(query) })
    }
}

fn item_error(error: rmac_app_launch::ItemError) -> BackendError {
    BackendError::new(FailureKind::Other, error.to_string())
}
