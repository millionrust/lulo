//! Dragging files out of a Lulo surface to another app on Windows (the
//! desktop's icons to a window, as Lulo OS does through Wayland's drag):
//! Windows' own OLE drag, with the shell's data object for the files, so
//! any app that takes a drop from Explorer takes these.

use std::path::PathBuf;

use windows::core::HSTRING;
use windows::Win32::Foundation::{
    DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, S_OK,
};
use windows::Win32::System::Com::{IBindCtx, IDataObject};
use windows::Win32::System::Ole::{
    DoDragDrop, IDropSource, IDropSource_Impl, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK,
    DROPEFFECT_MOVE,
};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    BHID_DataObject, ILFree, SHCreateShellItemArrayFromIDLists, SHParseDisplayName,
};

#[windows_core::implement(IDropSource)]
struct Source;

impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(
        &self,
        escape: windows::core::BOOL,
        keys: MODIFIERKEYS_FLAGS,
    ) -> windows::core::HRESULT {
        if escape.as_bool() {
            DRAGDROP_S_CANCEL
        } else if keys.0 & MK_LBUTTON.0 == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }

    fn GiveFeedback(&self, _effect: DROPEFFECT) -> windows::core::HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// Drag `paths` with the left button still held, until it is released over
/// a target or Esc cancels. Blocking: Windows runs the drag's own loop, so
/// call it outside any GPUI update (it pumps the windows' messages). True
/// when a target took the files.
pub fn drag_files(paths: &[PathBuf]) -> bool {
    // SAFETY: each PIDL comes from SHParseDisplayName and is freed below;
    // the array, data object and source are COM objects this function owns.
    unsafe {
        let mut pidls: Vec<*mut ITEMIDLIST> = Vec::new();
        for path in paths {
            let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
            let name = HSTRING::from(path.as_os_str());
            if SHParseDisplayName(&name, None::<&IBindCtx>, &mut pidl, 0, None).is_ok()
                && !pidl.is_null()
            {
                pidls.push(pidl);
            }
        }
        let dropped = (|| {
            if pidls.is_empty() {
                return false;
            }
            let list: Vec<*const ITEMIDLIST> = pidls.iter().map(|pidl| *pidl as *const _).collect();
            let Ok(items) = SHCreateShellItemArrayFromIDLists(&list) else {
                return false;
            };
            let Ok(data) =
                items.BindToHandler::<_, IDataObject>(None::<&IBindCtx>, &BHID_DataObject)
            else {
                return false;
            };
            let source: IDropSource = Source.into();
            let mut effect = DROPEFFECT(0);
            let result = DoDragDrop(
                &data,
                &source,
                DROPEFFECT(DROPEFFECT_COPY.0 | DROPEFFECT_MOVE.0 | DROPEFFECT_LINK.0),
                &mut effect,
            );
            result == DRAGDROP_S_DROP && effect.0 != 0
        })();
        for pidl in pidls {
            ILFree(Some(pidl as *const _));
        }
        dropped
    }
}
