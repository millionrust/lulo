//! rmac Text Editor — a fast, native TextEdit-style editor.

mod document;
mod long_lines;
mod recovery;
mod rtf;
mod storage;
#[cfg(test)]
mod test_alloc;
mod view;

gpui::actions!(
    text_editor,
    [
        NewFile,
        OpenFile,
        SaveFile,
        SaveFileAs,
        ExportPdf,
        PrintFile,
        ToggleFind,
        ToggleReplace,
        FindNext,
        FindPrev,
        CloseBar,
        ToggleMono,
        SetEncodingUtf8,
        SetEncodingUtf8Bom,
        SetEncodingUtf16Le,
        SetEncodingUtf16Be,
        SetLineEndingLf,
        SetLineEndingCrLf,
        SetLineEndingCr,
        IncreaseFont,
        DecreaseFont,
        CloseWindow,
    ]
);

fn main() {
    view::run();
}
