//! rmac Text Editor — a fast, native TextEdit-style editor.

mod document;
mod long_lines;
mod recovery;
mod rtf;
mod settings;
mod settings_window;
mod storage;
#[cfg(test)]
mod test_alloc;
mod view;

gpui::actions!(
    text_editor,
    [
        NewFile,
        ShowSettings,
        OpenFile,
        SaveFile,
        SaveFileAs,
        DuplicateDocument,
        ExportPdf,
        PrintFile,
        ToggleFind,
        ToggleReplace,
        FindNext,
        FindPrev,
        UseSelectionForFind,
        JumpToSelection,
        SaveGoToFolder,
        CloseBar,
        ToggleMono,
        ToggleWrapToPage,
        SetEncodingUtf8,
        SetEncodingUtf8Bom,
        SetEncodingUtf16Le,
        SetEncodingUtf16Be,
        SetLineEndingLf,
        SetLineEndingCrLf,
        SetLineEndingCr,
        IncreaseFont,
        DecreaseFont,
        ActualSize,
        CloseWindow,
        // File ▸ Open Recent ▸ (TE-02): one action per shown row, up to
        // `rmac_app_menu::recent::MAX_ENTRIES`, plus "Clear Menu".
        OpenRecent0,
        OpenRecent1,
        OpenRecent2,
        OpenRecent3,
        OpenRecent4,
        OpenRecent5,
        OpenRecent6,
        OpenRecent7,
        OpenRecent8,
        OpenRecent9,
        ClearRecentMenu,
        SheetWhereDocuments,
        SheetWhereDesktop,
        SheetWhereHome,
        SheetWhereDownloads,
        SheetEncodingUtf8,
        SheetEncodingUtf8Bom,
        SheetEncodingUtf16Le,
        SheetEncodingUtf16Be,
    ]
);

fn main() {
    view::run();
}
