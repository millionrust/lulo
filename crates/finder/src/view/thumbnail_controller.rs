use super::*;

impl FinderView {
    /// Generate platform thumbnails into the persistent cache off the main thread.
    pub(super) fn gen_thumbs(&mut self, cx: &mut Context<Self>) {
        let directory = self.cwd.clone();
        let targets: Vec<PathBuf> = self
            .entries
            .iter()
            .filter(|entry| {
                !entry.is_dir
                    && rmac_thumbnails::is_supported(&entry.path)
                    && !self.thumbs.get(&entry.path).is_some_and(|thumbnail| {
                        rmac_thumbnails::is_current(&entry.path, thumbnail)
                    })
            })
            .map(|e| e.path.clone())
            .collect();
        if targets.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let results = cx
                .background_executor()
                .spawn(async move {
                    let mut generated = Vec::new();
                    let mut first_error = None;
                    let mut failure_count = 0;
                    for path in targets {
                        match rmac_thumbnails::generate(&path) {
                            Ok(thumbnail) => generated.push((path, thumbnail)),
                            Err(error) => {
                                failure_count += 1;
                                first_error.get_or_insert(error);
                            }
                        }
                    }
                    (generated, first_error, failure_count)
                })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                for (p, t) in results.0 {
                    if this.entries.iter().any(|entry| entry.path == p)
                        && rmac_thumbnails::is_current(&p, &t)
                    {
                        this.thumbs.insert(p, t);
                    }
                }
                if this.cwd == directory && this.operation_error.is_none() {
                    this.operation_error = results.1.map(|error| {
                        if results.2 == 1 {
                            format!("Could not generate thumbnail: {error}").into()
                        } else {
                            format!(
                                "Could not generate thumbnail: {error} (and {} more failures)",
                                results.2 - 1
                            )
                            .into()
                        }
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }
}
