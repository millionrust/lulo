use super::*;

impl NotesView {
    pub(super) fn start_workers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let notes_paths = match resolve_notes_paths() {
            Ok(paths) => Some(paths),
            Err(error) => {
                self.message = Some(error.to_string().into());
                None
            }
        };

        if let Some(paths) = notes_paths.as_ref() {
            match NotesWorker::start(paths.clone())
                .map_err(|error| error.to_string())
                .and_then(|worker| {
                    let (client, events) = worker.into_parts();
                    worker_bridge::bridge_worker_events(events, EVENT_CAPACITY)
                        .map(|receiver| (client, receiver))
                        .map_err(|error| format!("Notes could not start its event bridge: {error}"))
                }) {
                Ok((client, receiver)) => {
                    self.worker = Some(client);
                    cx.spawn_in(window, async move |this, cx| {
                        while let Ok(event) = receiver.recv().await {
                            if this
                                .update_in(cx, |this, window, cx| {
                                    this.apply_worker_event(event, window, cx)
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                    })
                    .detach();
                }
                Err(message) => self.message = Some(message.into()),
            }

            if let Ok((client, receiver)) =
                NotesPreviewWorker::start(paths.data_root().to_path_buf())
                    .map_err(|error| error.to_string())
                    .and_then(|worker| {
                        let (client, events) = worker.into_parts();
                        worker_bridge::bridge_preview_events(
                            events,
                            PREVIEW_EVENT_CAPACITY,
                            worker_bridge::render_preview_image,
                        )
                        .map(|receiver| (client, receiver))
                        .map_err(|error| {
                            format!("Notes could not start its preview bridge: {error}")
                        })
                    })
            {
                self.preview_worker = Some(client);
                cx.spawn_in(window, async move |this, cx| {
                    while let Ok(event) = receiver.recv().await {
                        if this
                            .update_in(cx, |this, _window, cx| this.apply_preview_event(event, cx))
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .detach();
            }
        }

        match NotesMarkdownPreviewWorker::start()
            .map_err(|error| error.to_string())
            .and_then(|worker| {
                let (client, events) = worker.into_parts();
                worker_bridge::bridge_markdown_preview_events(
                    events,
                    MARKDOWN_PREVIEW_EVENT_CAPACITY,
                )
                .map(|receiver| (client, receiver))
                .map_err(|error| {
                    format!("Notes could not start its Markdown preview bridge: {error}")
                })
            }) {
            Ok((client, receiver)) => {
                self.markdown_preview_worker = Some(client);
                cx.spawn_in(window, async move |this, cx| {
                    while let Ok(event) = receiver.recv().await {
                        if this
                            .update_in(cx, |this, _window, cx| {
                                this.apply_markdown_preview_event(event, cx)
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .detach();
            }
            Err(message) => {
                if self.message.is_none() {
                    self.message = Some(message.into());
                }
            }
        }

        match NotesSearchWorker::start()
            .map_err(|error| error.to_string())
            .and_then(|worker| {
                let (client, events) = worker.into_parts();
                worker_bridge::bridge_search_events(events, SEARCH_EVENT_CAPACITY)
                    .map(|receiver| (client, receiver))
                    .map_err(|error| format!("Notes could not start its search bridge: {error}"))
            }) {
            Ok((client, receiver)) => {
                self.search_worker = Some(client);
                cx.spawn_in(window, async move |this, cx| {
                    while let Ok(event) = receiver.recv().await {
                        if this
                            .update_in(cx, |this, window, cx| {
                                this.apply_search_event(event, window, cx)
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .detach();
            }
            Err(message) => {
                if self.message.is_none() {
                    self.message = Some(message.into());
                }
            }
        }
    }
}
