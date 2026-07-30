//! Platform probe interaction controller.

use super::*;

pub(super) struct PlatformLab {
    input: Entity<InputState>,
    pub(super) event: SharedString,
    file_selected: bool,
    dropped_path_count: usize,
    clipboard_result: ClipboardProbeResult,
    probe_results: [ProbeResult; CAPABILITY_COUNT],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ClipboardProbeResult {
    #[default]
    NotRead,
    ExactProbe,
    OtherText,
    NoText,
}

impl ClipboardProbeResult {
    pub(super) fn summary(self) -> &'static str {
        match self {
            Self::NotRead => "No clipboard text read",
            Self::ExactProbe => "Built-in clipboard probe matched exactly",
            Self::OtherText => "Clipboard text received; content hidden",
            Self::NoText => "Clipboard did not contain text",
        }
    }
}

pub(super) fn selected_file_summary(selected: bool) -> &'static str {
    if selected {
        "One file selected; path hidden"
    } else {
        "No file selected"
    }
}

pub(super) fn dropped_paths_summary(count: usize) -> String {
    match count {
        0 => "No external files dropped".to_string(),
        1 => "One external path received; path hidden".to_string(),
        count => format!("{count} external paths received; paths hidden"),
    }
}

impl PlatformLab {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Type here: English, हिंदी, 中文, emoji, compose keys…")
        });

        cx.observe(&input, |this, _, cx| {
            this.event = "Text input changed".into();
            cx.notify();
        })
        .detach();

        cx.bind_keys([
            KeyBinding::new("cmd-shift-c", CopyProbe, Some("PlatformLab")),
            KeyBinding::new("ctrl-shift-c", CopyProbe, Some("PlatformLab")),
            KeyBinding::new("cmd-shift-v", ReadProbe, Some("PlatformLab")),
            KeyBinding::new("ctrl-shift-v", ReadProbe, Some("PlatformLab")),
            KeyBinding::new("cmd-o", OpenProbe, Some("PlatformLab")),
            KeyBinding::new("ctrl-o", OpenProbe, Some("PlatformLab")),
        ]);

        Self {
            input,
            event: "Ready — work through every probe and record the result".into(),
            file_selected: false,
            dropped_path_count: 0,
            clipboard_result: ClipboardProbeResult::NotRead,
            probe_results: [ProbeResult::Pending; CAPABILITY_COUNT],
        }
    }

    pub(super) fn copy_probe(&mut self, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(CLIPBOARD_PROBE.to_string()));
        self.event = "Clipboard probe written".into();
        cx.notify();
    }

    pub(super) fn read_probe(&mut self, cx: &mut Context<Self>) {
        self.clipboard_result = match cx.read_from_clipboard().and_then(|item| item.text()) {
            Some(text) if text == CLIPBOARD_PROBE => ClipboardProbeResult::ExactProbe,
            Some(_) => ClipboardProbeResult::OtherText,
            None => ClipboardProbeResult::NoText,
        };
        self.event = self.clipboard_result.summary().into();
        cx.notify();
    }

    pub(super) fn open_probe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Select probe file".into()),
        });
        self.event = "File chooser opened".into();
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let outcome = receiver.await;
            let _ = this.update_in(cx, |this, _, cx| {
                match outcome {
                    Ok(Ok(Some(paths))) => {
                        this.file_selected = !paths.is_empty();
                        this.event = "File chooser returned a path".into();
                    }
                    Ok(Ok(None)) => {
                        this.file_selected = false;
                        this.event = "File chooser cancelled".into();
                    }
                    Ok(Err(_)) => this.event = "File chooser returned an error".into(),
                    Err(_) => this.event = "File chooser channel closed unexpectedly".into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_probe_result(
        &mut self,
        index: usize,
        result: ProbeResult,
        cx: &mut Context<Self>,
    ) {
        let Some(slot) = self.probe_results.get_mut(index) else {
            self.event = "Ignored an invalid evidence-result index".into();
            cx.notify();
            return;
        };
        *slot = result;
        let recorded = recorded_result_count(&self.probe_results);
        self.event = format!("Recorded {recorded} of {CAPABILITY_COUNT} evidence results").into();
        cx.notify();
    }

    pub(super) fn copy_evidence_report(&mut self, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(evidence_report(
            &self.probe_results,
        )));
        let recorded = recorded_result_count(&self.probe_results);
        self.event =
            format!("Redacted report copied — {recorded} of {CAPABILITY_COUNT} recorded").into();
        cx.notify();
    }

    pub(super) fn reset_evidence_report(&mut self, cx: &mut Context<Self>) {
        self.probe_results.fill(ProbeResult::Pending);
        self.event = "Evidence results reset to pending".into();
        cx.notify();
    }

    pub(super) fn render_probe_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_1()
            .min_w(px(0.0))
            .v_flex()
            .gap_3()
            .child(section_title("Interactive probes"))
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(label("Text input and IME"))
                    .child(
                        div()
                            .border_1()
                            .border_color(mac::separator())
                            .rounded(px(7.0))
                            .px_2()
                            .py_1()
                            .child(Input::new(&self.input)),
                    ),
            )
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(label("Clipboard"))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("copy-probe")
                                    .label("Copy probe")
                                    .primary()
                                    .with_size(gpui_component::Size::Small)
                                    .on_click(cx.listener(|this, _, _, cx| this.copy_probe(cx))),
                            )
                            .child(
                                Button::new("read-probe")
                                    .label("Read clipboard")
                                    .with_size(gpui_component::Size::Small)
                                    .on_click(cx.listener(|this, _, _, cx| this.read_probe(cx))),
                            ),
                    )
                    .child(value(self.clipboard_result.summary())),
            )
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(label("File chooser"))
                    .child(
                        Button::new("open-probe")
                            .label("Choose a file…")
                            .with_size(gpui_component::Size::Small)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_probe(window, cx)),
                            ),
                    )
                    .child(value(selected_file_summary(self.file_selected))),
            )
            .child(
                div()
                    .id("external-drop-target")
                    .h(px(92.0))
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .border_2()
                    .border_color(gpui::rgb(0x5aa9e6))
                    .rounded(px(9.0))
                    .bg(gpui::rgb(0xeaf5ff))
                    .text_size(px(12.0))
                    .text_color(mac::text_secondary())
                    .drag_over::<ExternalPaths>(|style, _, _, _| style.bg(gpui::rgb(0xcfeaff)))
                    .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                        this.dropped_path_count = paths.paths().len();
                        this.event =
                            format!("Received {} dropped path(s)", paths.paths().len()).into();
                        cx.notify();
                    }))
                    .child("Drop files here from the system file manager")
                    .child(value(dropped_paths_summary(self.dropped_path_count))),
            )
            .child(label("Scroll stress list"))
            .child(
                div()
                    .id("scroll-probe")
                    .h(px(180.0))
                    .w_full()
                    .overflow_y_scroll()
                    .border_1()
                    .border_color(mac::separator())
                    .rounded(px(7.0))
                    .children((1..=120).map(|row| {
                        div()
                            .h(px(28.0))
                            .flex()
                            .items_center()
                            .px_3()
                            .bg(if row % 2 == 0 {
                                gpui::rgb(0xf7f7f7).into()
                            } else {
                                mac::window()
                            })
                            .text_size(px(12.0))
                            .child(format!("Row {row:03} — smooth scroll and scale probe"))
                    })),
            )
    }

    pub(super) fn render_capabilities(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let recorded = recorded_result_count(&self.probe_results);
        div()
            .id("capability-results")
            .w(px(390.0))
            .min_w(px(300.0))
            .h_full()
            .overflow_y_scroll()
            .v_flex()
            .gap_3()
            .child(section_title("Stable GPUI 0.2.2 capability gate"))
            .child(value(format!(
                "{recorded} of {CAPABILITY_COUNT} results recorded; pending is never a pass"
            )))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("copy-evidence-report")
                            .label("Copy redacted report")
                            .with_size(gpui_component::Size::Small)
                            .on_click(cx.listener(|this, _, _, cx| this.copy_evidence_report(cx))),
                    )
                    .child(
                        Button::new("reset-evidence-report")
                            .label("Reset")
                            .with_size(gpui_component::Size::Small)
                            .on_click(cx.listener(|this, _, _, cx| this.reset_evidence_report(cx))),
                    ),
            )
            .children(CAPABILITIES.iter().enumerate().map(|(index, capability)| {
                let result = self.probe_results[index];
                let positive_label = match capability.status {
                    CapabilityStatus::ExerciseHere => "Pass",
                    CapabilityStatus::MissingFromStableApi => "Confirm blocker",
                };
                div()
                    .v_flex()
                    .gap_1()
                    .p_3()
                    .border_1()
                    .border_color(mac::separator())
                    .rounded(px(8.0))
                    .bg(mac::window())
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .font_weight(mac::SEMIBOLD)
                                    .text_color(mac::text())
                                    .child(capability.name),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        div()
                                            .px_2()
                                            .py(px(2.0))
                                            .rounded_full()
                                            .bg(capability.status.color())
                                            .text_color(gpui::white())
                                            .text_size(px(9.0))
                                            .font_weight(mac::BOLD)
                                            .child(capability.status.label()),
                                    )
                                    .child(
                                        div()
                                            .px_2()
                                            .py(px(2.0))
                                            .rounded_full()
                                            .bg(result.color())
                                            .text_color(gpui::white())
                                            .text_size(px(9.0))
                                            .font_weight(mac::BOLD)
                                            .child(result.label()),
                                    ),
                            ),
                    )
                    .child(value(capability.instruction))
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(
                                Button::new(SharedString::from(format!("probe-positive-{index}")))
                                    .label(positive_label)
                                    .with_size(gpui_component::Size::XSmall)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.set_probe_result(
                                            index,
                                            positive_result(CAPABILITIES[index]),
                                            cx,
                                        )
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(format!("probe-failed-{index}")))
                                    .label("Fail")
                                    .with_size(gpui_component::Size::XSmall)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.set_probe_result(index, ProbeResult::Failed, cx)
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(format!("probe-pending-{index}")))
                                    .label("Pending")
                                    .with_size(gpui_component::Size::XSmall)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.set_probe_result(index, ProbeResult::Pending, cx)
                                    })),
                            ),
                    )
            }))
    }
}
