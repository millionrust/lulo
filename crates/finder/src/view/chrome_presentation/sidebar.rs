use super::*;

impl FinderView {
    fn render_place(&self, p: &Place, cx: &Context<Self>) -> impl IntoElement {
        let is_tag = p.kind == PlaceKind::Tag;
        let selected = if p.kind == PlaceKind::Trash {
            self.trash_view
        } else {
            !is_tag && !self.trash_view && self.cwd == p.path
        };
        let key = format!("{}-{}", p.name, p.path.display());

        let leading: gpui::AnyElement = if is_tag {
            div()
                .w(px(12.0))
                .h(px(12.0))
                .flex_none()
                .rounded_full()
                .bg(p.tint)
                .into_any_element()
        } else {
            icon(p.icon, 17.0, if selected { accent() } else { label() }).into_any_element()
        };

        let np = p.path.clone();
        let tag_name = p.name.clone();
        let kind = p.kind;
        let main = div()
            .flex_1()
            .flex()
            .items_center()
            .gap_2()
            .min_w(px(0.0))
            .child(leading)
            .child(
                Button::new(
                    SharedString::from(format!("placemain-{key}")),
                    p.name.clone(),
                )
                .ghost()
                .xsmall()
                .selected(selected)
                .flex_1()
                .on_click(cx.listener(move |this, _, _, cx| match kind {
                    PlaceKind::Tag => this.tag_click(tag_name.clone(), cx),
                    PlaceKind::Recents => this.recents_click(cx),
                    PlaceKind::Trash => this.trash_click(cx),
                    _ => this.navigate(np.clone(), cx),
                })),
            );

        let mut row = div()
            .id(SharedString::from(format!("place-{key}")))
            .flex()
            .items_center()
            .gap_2()
            .h(px(28.0))
            .px_2()
            .rounded(px(6.0))
            .when(selected, |el: Stateful<Div>| {
                el.bg(rmac_ui::mac::sidebar_selection())
            })
            .when(!selected && !is_tag, |el: Stateful<Div>| {
                el.hover(|h| h.bg(rmac_ui::mac::hover()))
            })
            .child(main);

        if p.kind == PlaceKind::Volume {
            let ep = p.path.clone();
            let tooltip = format!("Eject {}", p.name);
            row = row.child(
                Button::new(SharedString::from(format!("eject-{key}")), "Eject")
                    .ghost()
                    .xsmall()
                    .tooltip(tooltip)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.eject_volume(ep.clone(), cx);
                    })),
            );
        }
        row
    }

    pub(in crate::view) fn trash_click(&mut self, cx: &mut Context<Self>) {
        self.trash_view = true;
        if self.view == ViewMode::Column {
            self.view = ViewMode::List;
        }
        self.result_title = Some("Trash".into());
        self.operation_error = None;
        self.reload_trash(cx);
    }

    pub(in crate::view) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div()
            .w(px(self.sidebar_width))
            .h_full()
            .flex_shrink_0()
            .v_flex()
            .relative()
            .pt_2()
            .px_2()
            .gap_0p5()
            .bg(sidebar_bg())
            .border_r_1()
            .border_color(sep());
        for (si, section) in self.sections.iter().enumerate() {
            col = col.child(
                div()
                    .px_2()
                    .pt(px(if si == 0 { 2.0 } else { 12.0 }))
                    .pb_1()
                    .text_size(rmac_ui::text_px(11.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(secondary())
                    .child(section.title.clone()),
            );
            for p in &section.places {
                col = col.child(self.render_place(p, cx));
            }
        }
        col.child(
            div()
                .id("sidebar-resizer")
                .absolute()
                .right_0()
                .top_0()
                .bottom_0()
                .w(px(5.0))
                .hover(|handle| handle.bg(rmac_ui::mac::accent_subtle()))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| this.begin_sidebar_resize()),
                )
                .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                    if event.pressed_button == Some(MouseButton::Left) {
                        this.resize_sidebar(f32::from(event.position.x), cx);
                    }
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| this.finish_sidebar_resize()),
                ),
        )
    }
}
