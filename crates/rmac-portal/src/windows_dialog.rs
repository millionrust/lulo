//! The Windows common Open dialog (`IFileOpenDialog`), standing in for the
//! desktop portal's file chooser on Windows (ADR 0023 phase 2f).
//!
//! The dialog is modal and needs a COM apartment of its own, so it runs on a
//! thread it owns and the caller awaits the answer; the UI thread never
//! blocks on it.

use std::path::PathBuf;

use windows::core::{HRESULT, PCWSTR};
use windows::Win32::Foundation::ERROR_CANCELLED;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOLDERID_Documents, FileOpenDialog, IFileOpenDialog, IShellItem, SHGetKnownFolderItem,
    FOS_ALLOWMULTISELECT, FOS_FORCEFILESYSTEM, KF_FLAG_DEFAULT, SIGDN_FILESYSPATH,
};

use crate::{Error, Operation};

/// One entry of the "files of type" list: a label and the patterns it shows,
/// `;`-separated (`"*.mp3;*.mp4"`).
pub struct Filter {
    pub name: &'static str,
    pub patterns: &'static str,
}

fn failure(detail: &str) -> Error {
    Error {
        operation: Operation::Choose,
        path: PathBuf::new(),
        detail: detail.into(),
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn show(title: &str, filters: &[Filter], multiple: bool) -> windows::core::Result<Vec<PathBuf>> {
    // SAFETY: standard COM usage on a thread that initialised its apartment;
    // every buffer handed to the dialog outlives the call that uses it.
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        let title = wide(title);
        dialog.SetTitle(PCWSTR(title.as_ptr()))?;
        let mut options = dialog.GetOptions()? | FOS_FORCEFILESYSTEM;
        if multiple {
            options |= FOS_ALLOWMULTISELECT;
        }
        dialog.SetOptions(options)?;
        // With no folder remembered yet, Windows 11 opens the dialog on Home
        // (Quick access and "Recommended" cloud files), which can keep the
        // created dialog hidden for half a minute while it asks the cloud,
        // and the hidden dialog already holds the keyboard. Start in
        // Documents instead, as the Mac's Open panel does; a folder the
        // dialog remembers from last time still wins.
        if let Ok(documents) =
            SHGetKnownFolderItem::<IShellItem>(&FOLDERID_Documents, KF_FLAG_DEFAULT, None)
        {
            let _ = dialog.SetDefaultFolder(&documents);
        }
        let names: Vec<Vec<u16>> = filters.iter().map(|filter| wide(filter.name)).collect();
        let patterns: Vec<Vec<u16>> = filters.iter().map(|filter| wide(filter.patterns)).collect();
        let specs: Vec<COMDLG_FILTERSPEC> = names
            .iter()
            .zip(&patterns)
            .map(|(name, spec)| COMDLG_FILTERSPEC {
                pszName: PCWSTR(name.as_ptr()),
                pszSpec: PCWSTR(spec.as_ptr()),
            })
            .collect();
        if !specs.is_empty() {
            dialog.SetFileTypes(&specs)?;
        }
        if let Err(error) = dialog.Show(None) {
            if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) {
                return Ok(Vec::new());
            }
            return Err(error);
        }
        let results = dialog.GetResults()?;
        let mut paths = Vec::new();
        for index in 0..results.GetCount()? {
            let item = results.GetItemAt(index)?;
            let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
            paths.push(PathBuf::from(name.to_string().unwrap_or_default()));
            CoTaskMemFree(Some(name.0.cast()));
        }
        Ok(paths)
    }
}

/// Ask for files: an empty list when the person cancels.
pub async fn open_files(
    title: &'static str,
    filters: &'static [Filter],
    multiple: bool,
) -> Result<Vec<PathBuf>, Error> {
    let (sender, receiver) = async_channel::bounded(1);
    let spawned = std::thread::Builder::new()
        .name("lulo-open-dialog".into())
        .spawn(move || {
            // SAFETY: balanced with `CoUninitialize` below.
            let initialised = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
            let answer = show(title, filters, multiple);
            if initialised {
                // SAFETY: this thread initialised COM above.
                unsafe { CoUninitialize() };
            }
            let _ = sender.send_blocking(answer);
        });
    if spawned.is_err() {
        return Err(failure("the Open dialog could not start"));
    }
    match receiver.recv().await {
        Ok(Ok(paths)) => Ok(paths
            .into_iter()
            .filter(|path| path.is_absolute())
            .collect()),
        Ok(Err(_)) | Err(_) => Err(failure("the Open dialog could not be shown")),
    }
}
