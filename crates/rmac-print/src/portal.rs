use std::fmt;

use crate::{Error as RenderError, PageDescription, PageLayout};

const MAX_PARENT_WINDOW_BYTES: usize = 512;
const MAX_TITLE_BYTES: usize = 256;
const SUPPORTED_OUTPUT_FILE_FORMATS: [&str; 1] = ["pdf"];

/// Identity retained across the two portal calls and the intervening render.
/// Generations prevent a response for an old document or recycled window from
/// being applied to the currently visible document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrintIdentity {
    parent_window: String,
    window_generation: u64,
    document_generation: u64,
}

impl PrintIdentity {
    pub fn new(
        parent_window: impl Into<String>,
        window_generation: u64,
        document_generation: u64,
    ) -> Result<Self, PortalPrintError> {
        let parent_window = parent_window.into();
        let handle = parent_window
            .strip_prefix("wayland:")
            .ok_or(PortalPrintError::InvalidParentWindow)?;
        if handle.is_empty()
            || parent_window.len() > MAX_PARENT_WINDOW_BYTES
            || parent_window.chars().any(char::is_control)
        {
            return Err(PortalPrintError::InvalidParentWindow);
        }
        Ok(Self {
            parent_window,
            window_generation,
            document_generation,
        })
    }

    pub fn parent_window(&self) -> &str {
        &self.parent_window
    }

    pub fn window_generation(&self) -> u64 {
        self.window_generation
    }

    pub fn document_generation(&self) -> u64 {
        self.document_generation
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    Pdf,
}

impl OutputFormat {
    pub fn from_portal(value: &str) -> Option<Self> {
        value.eq_ignore_ascii_case("pdf").then_some(Self::Pdf)
    }

    pub fn portal_option(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortalPrintPhase {
    Preparing,
    Rendering,
    Ready,
    Submitting,
    Finished,
    Cancelled,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortalPrintError {
    InvalidParentWindow,
    InvalidTitle,
    StaleIdentity,
    WrongPhase,
    UnsupportedOutputFormat,
    InvalidPageSetup,
}

impl fmt::Display for PortalPrintError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidParentWindow => "printing requires the current Wayland window identity",
            Self::InvalidTitle => "the print job title is invalid",
            Self::StaleIdentity => "the document or window changed while preparing to print",
            Self::WrongPhase => "the print portal returned an unexpected transaction step",
            Self::UnsupportedOutputFormat => {
                "the print portal selected a format this application cannot produce"
            }
            Self::InvalidPageSetup => "the print portal returned an unsupported page setup",
        })
    }
}

impl std::error::Error for PortalPrintError {}

#[derive(Clone, Copy, Debug)]
enum State {
    Preparing,
    Rendering { token: u32, layout: PageLayout },
    Ready { token: u32 },
    Submitting,
    Finished,
    Cancelled,
    Failed,
}

/// Pure transaction boundary for XDG Print's PreparePrint → render → Print
/// sequence. Portal transport, exported Wayland handles, and readable file
/// descriptors stay in the platform adapter; ordering and stale-response
/// protection remain testable on every development host.
#[derive(Clone, Debug)]
pub struct PortalPrintTransaction {
    identity: PrintIdentity,
    title: String,
    state: State,
}

impl PortalPrintTransaction {
    pub fn new(
        identity: PrintIdentity,
        title: impl Into<String>,
    ) -> Result<Self, PortalPrintError> {
        let title = title.into();
        if title.is_empty() || title.len() > MAX_TITLE_BYTES || title.chars().any(char::is_control)
        {
            return Err(PortalPrintError::InvalidTitle);
        }
        Ok(Self {
            identity,
            title,
            state: State::Preparing,
        })
    }

    pub fn phase(&self) -> PortalPrintPhase {
        match self.state {
            State::Preparing => PortalPrintPhase::Preparing,
            State::Rendering { .. } => PortalPrintPhase::Rendering,
            State::Ready { .. } => PortalPrintPhase::Ready,
            State::Submitting => PortalPrintPhase::Submitting,
            State::Finished => PortalPrintPhase::Finished,
            State::Cancelled => PortalPrintPhase::Cancelled,
            State::Failed => PortalPrintPhase::Failed,
        }
    }

    pub fn identity(&self) -> &PrintIdentity {
        &self.identity
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// Exact formats to pass as `supported_output_file_formats` to both portal
    /// calls. Omitting this option would falsely claim PS and SVG support.
    pub fn supported_output_file_formats(&self) -> &'static [&'static str] {
        &SUPPORTED_OUTPUT_FILE_FORMATS
    }

    pub fn prepared(
        &mut self,
        current: &PrintIdentity,
        token: u32,
        page: PageDescription,
        selected_output_file_format: Option<&str>,
    ) -> Result<PageLayout, PortalPrintError> {
        self.require_identity(current)?;
        if !matches!(self.state, State::Preparing) {
            return Err(PortalPrintError::WrongPhase);
        }
        if selected_output_file_format
            .is_some_and(|format| OutputFormat::from_portal(format).is_none())
        {
            self.state = State::Failed;
            return Err(PortalPrintError::UnsupportedOutputFormat);
        }
        let layout = match PageLayout::from_description(page) {
            Ok(layout) => layout,
            Err(RenderError::InvalidPageLayout) => {
                self.state = State::Failed;
                return Err(PortalPrintError::InvalidPageSetup);
            }
            Err(_) => unreachable!("page-description conversion only validates layout"),
        };
        self.state = State::Rendering { token, layout };
        Ok(layout)
    }

    pub fn rendered(
        &mut self,
        current: &PrintIdentity,
        layout: PageLayout,
        output: OutputFormat,
    ) -> Result<(), PortalPrintError> {
        self.require_identity(current)?;
        let State::Rendering {
            token,
            layout: prepared_layout,
        } = self.state
        else {
            return Err(PortalPrintError::WrongPhase);
        };
        if prepared_layout != layout || output != OutputFormat::Pdf {
            self.state = State::Failed;
            return Err(PortalPrintError::InvalidPageSetup);
        }
        self.state = State::Ready { token };
        Ok(())
    }

    pub fn submission(
        &mut self,
        current: &PrintIdentity,
    ) -> Result<PrintSubmission, PortalPrintError> {
        self.require_identity(current)?;
        let State::Ready { token } = self.state else {
            return Err(PortalPrintError::WrongPhase);
        };
        self.state = State::Submitting;
        Ok(PrintSubmission {
            parent_window: self.identity.parent_window.clone(),
            title: self.title.clone(),
            token,
            output_format: OutputFormat::Pdf,
        })
    }

    pub fn finish(&mut self, current: &PrintIdentity) -> Result<(), PortalPrintError> {
        self.require_identity(current)?;
        if !matches!(self.state, State::Submitting) {
            return Err(PortalPrintError::WrongPhase);
        }
        self.state = State::Finished;
        Ok(())
    }

    /// Cancellation is a normal terminal result from either portal request.
    pub fn cancel(&mut self, current: &PrintIdentity) -> Result<(), PortalPrintError> {
        self.require_identity(current)?;
        if matches!(
            self.state,
            State::Finished | State::Cancelled | State::Failed
        ) {
            return Err(PortalPrintError::WrongPhase);
        }
        self.state = State::Cancelled;
        Ok(())
    }

    /// Transport failures are terminal but keep private bus diagnostics out of
    /// the domain and user-facing error surface.
    pub fn fail(&mut self, current: &PrintIdentity) -> Result<(), PortalPrintError> {
        self.require_identity(current)?;
        if matches!(
            self.state,
            State::Finished | State::Cancelled | State::Failed
        ) {
            return Err(PortalPrintError::WrongPhase);
        }
        self.state = State::Failed;
        Ok(())
    }

    fn require_identity(&self, current: &PrintIdentity) -> Result<(), PortalPrintError> {
        (&self.identity == current)
            .then_some(())
            .ok_or(PortalPrintError::StaleIdentity)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrintSubmission {
    pub parent_window: String,
    pub title: String,
    pub token: u32,
    pub output_format: OutputFormat,
}

impl PrintSubmission {
    pub fn supported_output_file_formats(&self) -> &'static [&'static str] {
        &SUPPORTED_OUTPUT_FILE_FORMATS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PageOrientation;

    fn identity(document_generation: u64) -> PrintIdentity {
        PrintIdentity::new("wayland:~rmac-window-7", 7, document_generation).unwrap()
    }

    #[test]
    fn transaction_preserves_identity_layout_token_and_pdf_only_contract() {
        let identity = identity(12);
        let mut transaction = PortalPrintTransaction::new(identity.clone(), "Untitled").unwrap();
        assert_eq!(transaction.phase(), PortalPrintPhase::Preparing);
        assert_eq!(transaction.supported_output_file_formats(), &["pdf"]);

        let layout = transaction
            .prepared(
                &identity,
                44,
                PageDescription {
                    width_mm: Some(210.0),
                    height_mm: Some(297.0),
                    orientation: Some(PageOrientation::Landscape),
                    ..PageDescription::default()
                },
                Some("PDF"),
            )
            .unwrap();
        assert_eq!(transaction.phase(), PortalPrintPhase::Rendering);
        assert_eq!((layout.width_mm, layout.height_mm), (297.0, 210.0));

        transaction
            .rendered(&identity, layout, OutputFormat::Pdf)
            .unwrap();
        let submission = transaction.submission(&identity).unwrap();
        assert_eq!(submission.parent_window, "wayland:~rmac-window-7");
        assert_eq!(submission.title, "Untitled");
        assert_eq!(submission.token, 44);
        assert_eq!(submission.output_format, OutputFormat::Pdf);
        assert_eq!(submission.supported_output_file_formats(), &["pdf"]);
        transaction.finish(&identity).unwrap();
        assert_eq!(transaction.phase(), PortalPrintPhase::Finished);
    }

    #[test]
    fn stale_document_or_window_cannot_advance_a_transaction() {
        let original = identity(12);
        let current = identity(13);
        let mut transaction = PortalPrintTransaction::new(original, "Document.txt").unwrap();

        assert_eq!(
            transaction.prepared(&current, 1, PageDescription::default(), None),
            Err(PortalPrintError::StaleIdentity)
        );
        assert_eq!(transaction.phase(), PortalPrintPhase::Preparing);
    }

    #[test]
    fn unsupported_print_to_file_format_fails_closed() {
        let identity = identity(1);
        let mut transaction = PortalPrintTransaction::new(identity.clone(), "Draft").unwrap();

        assert_eq!(
            transaction.prepared(&identity, 1, PageDescription::default(), Some("PS")),
            Err(PortalPrintError::UnsupportedOutputFormat)
        );
        assert_eq!(transaction.phase(), PortalPrintPhase::Failed);
    }

    #[test]
    fn cancellation_is_normal_and_terminal_at_each_portal_boundary() {
        let identity = identity(1);
        let mut preparing = PortalPrintTransaction::new(identity.clone(), "Draft").unwrap();
        preparing.cancel(&identity).unwrap();
        assert_eq!(preparing.phase(), PortalPrintPhase::Cancelled);
        assert_eq!(
            preparing.cancel(&identity),
            Err(PortalPrintError::WrongPhase)
        );

        let mut submitting = PortalPrintTransaction::new(identity.clone(), "Draft").unwrap();
        let layout = submitting
            .prepared(&identity, 8, PageDescription::default(), None)
            .unwrap();
        submitting
            .rendered(&identity, layout, OutputFormat::Pdf)
            .unwrap();
        submitting.submission(&identity).unwrap();
        submitting.cancel(&identity).unwrap();
        assert_eq!(submitting.phase(), PortalPrintPhase::Cancelled);
    }

    #[test]
    fn invalid_or_unbound_windows_and_titles_are_refused() {
        assert_eq!(
            PrintIdentity::new("", 1, 1),
            Err(PortalPrintError::InvalidParentWindow)
        );
        assert_eq!(
            PrintIdentity::new("x11:1234", 1, 1),
            Err(PortalPrintError::InvalidParentWindow)
        );
        let identity = identity(1);
        assert!(matches!(
            PortalPrintTransaction::new(identity.clone(), ""),
            Err(PortalPrintError::InvalidTitle)
        ));
        assert!(matches!(
            PortalPrintTransaction::new(identity, "bad\ntitle"),
            Err(PortalPrintError::InvalidTitle)
        ));
    }

    #[test]
    fn phase_order_and_exact_render_layout_are_enforced() {
        let identity = identity(1);
        let mut transaction = PortalPrintTransaction::new(identity.clone(), "Draft").unwrap();
        assert!(matches!(
            transaction.submission(&identity),
            Err(PortalPrintError::WrongPhase)
        ));
        let layout = transaction
            .prepared(&identity, 2, PageDescription::default(), None)
            .unwrap();
        assert_eq!(
            transaction.rendered(
                &identity,
                PageLayout {
                    width_mm: 216.0,
                    ..layout
                },
                OutputFormat::Pdf,
            ),
            Err(PortalPrintError::InvalidPageSetup)
        );
        assert_eq!(transaction.phase(), PortalPrintPhase::Failed);
    }

    #[test]
    fn portal_orientation_spellings_map_to_renderer_geometry() {
        assert_eq!(
            PageOrientation::from_portal("reverse_landscape"),
            Some(PageOrientation::Landscape)
        );
        assert_eq!(
            PageOrientation::from_portal("reverse-portrait"),
            Some(PageOrientation::Portrait)
        );
        assert_eq!(PageOrientation::from_portal("sideways"), None);
    }
}
