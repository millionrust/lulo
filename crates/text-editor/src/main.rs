//! rmac Text Editor — a fast, native TextEdit-style editor.

mod document;
mod long_lines;
mod recovery;
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
        SelectLine,
        TransformUppercase,
        TransformLowercase,
        TransformCapitalise,
        InsertLineBreak,
        InsertParagraphBreak,
        InsertPageBreak,
        SaveGoToFolder,
        CloseBar,
        ToggleMono,
        ToggleWrapToPage,
        PreventEditing,
        SetEncodingUtf8,
        SetEncodingUtf8Bom,
        SetEncodingUtf16Le,
        SetEncodingUtf16Be,
        SetLineEndingLf,
        SetLineEndingCrLf,
        SetLineEndingCr,
        IncreaseFont,
        DecreaseFont,
        // View ▸ Zoom In / Zoom Out: the same scaling as Format ▸ Font ▸
        // Bigger / Smaller, under their own actions so each menu row is
        // distinct on the wire.
        ZoomIn,
        ZoomOut,
        ActualSize,
        EnterFullScreen,
        CloseWindow,
        CloseAll,
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
        ShowSpellingAndGrammar,
        CheckDocumentNow,
        ToggleCheckSpellingWhileTyping,
        ToggleCheckGrammarWithSpelling,
        ToggleCorrectSpellingAutomatically,
        ShowSubstitutions,
        ToggleSmartCopyPaste,
        ToggleSmartQuotes,
        ToggleSmartDashes,
        ToggleSmartLinks,
        ToggleDataDetectors,
        ToggleTextReplacement,
        StartSpeaking,
        StopSpeaking,
        // Application ▸ Quit and Keep Windows (TXT-MENU-001).
        QuitAndKeepWindows,
        // File ▸ Rename…/Move To…/Revert To ▸ Last Saved/Page Setup…
        // (TXT-MENU-002/003/004/006). Each sheet's own Cancel/Rename/OK and
        // radio rows are click-handled directly (like the Settings window's
        // controls), not dispatched as further menu-bar actions.
        RenameDocument,
        MoveToFolder,
        RevertToLastSaved,
        OpenPageSetup,
        // Format ▸ Make Rich Text / Make Plain Text (TXT-MENU-075).
        ToggleRichText,
        // Format ▸ Text ▸ alignment, ruler and spacing (TXT-MENU-060..074).
        AlignLeft,
        AlignCentre,
        AlignRight,
        ShowRuler,
        CopyRuler,
        PasteRuler,
        OpenSpacing,
        // View ▸ Use Dark Background for Windows (TXT-MENU-082).
        ToggleDarkBackground,
        // Format ▸ Font (rich text): character styles of the selection or
        // the next typing (TE-03).
        ToggleBold,
        ToggleItalic,
        ToggleUnderline,
        ShowColours,
        CopyStyle,
        PasteStyle,
        // Format ▸ Font ▸ Highlight ▸ …
        HighlightNone,
        HighlightAccent,
        HighlightPurple,
        HighlightPink,
        HighlightOrange,
        HighlightMint,
        HighlightBlue,
        // Format ▸ Text ▸ Justify.
        AlignJustify,
        // Format ▸ List…
        ShowLists,
        // Format ▸ Font ▸ Show Fonts (⌘T).
        ShowFonts,
        // Format ▸ Font ▸ Outline.
        ToggleOutline,
        // Format ▸ Font ▸ Kern ▸ …
        KernDefault,
        KernNone,
        KernTighten,
        KernLoosen,
        // Format ▸ Font ▸ Ligatures ▸ …
        LigaturesDefault,
        LigaturesNone,
        LigaturesAll,
        // Format ▸ Font ▸ Baseline ▸ …
        BaselineDefault,
        BaselineSuperscript,
        BaselineSubscript,
        BaselineRaise,
        BaselineLower,
        // Format ▸ Font ▸ Character Shape ▸ Traditional Form.
        ToggleTraditionalForm,
        // Format ▸ Allow Hyphenation.
        ToggleHyphenation,
        // File ▸ Show Properties (⌥⌘P).
        ShowProperties,
        // Edit ▸ Link… (⌘K).
        EditLink,
        // Format ▸ Font ▸ Styles….
        ShowStyles,
    ]
);

fn main() {
    view::run();
}
