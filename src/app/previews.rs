//! Previews behavior for the application.

use super::*;

impl App {
    /// Give the pointer time to leave a norloge and reach the preview panel
    /// before hiding it. `preview_hovered` cancels the hide when the pointer
    /// actually enters the panel.
    pub(super) fn schedule_preview_hide(&mut self, tasks: &mut Vec<Task<Message>>) {
        if self.hovered_preview.is_none() || self.preview_hovered {
            return;
        }
        self.preview_epoch = self.preview_epoch.wrapping_add(1);
        let epoch = self.preview_epoch;
        tasks.push(Task::perform(
            async {
                tokio::time::sleep(Duration::from_millis(700)).await;
            },
            move |_| Message::HidePreview(epoch),
        ));
    }

    /// Keep only the most recent title and image previews. Errors and pending
    /// fetches are cheap and stay cached so hovering a failed link keeps
    /// showing its red cross.
    pub(super) fn cache_link_preview(&mut self, url: String, preview: LinkPreview) {
        let is_image = match &preview {
            LinkPreview::Image { .. } => Some(true),
            LinkPreview::Title(_) => Some(false),
            LinkPreview::Loading | LinkPreview::Error => None,
        };
        if let Some(is_image) = is_image {
            let order = if is_image {
                &mut self.link_image_order
            } else {
                &mut self.link_title_order
            };
            order.retain(|item| item != &url);
            order.push_back(url.clone());
            while order.len() > LINK_PREVIEW_CACHE_LIMIT {
                if let Some(oldest) = order.pop_front() {
                    self.link_previews.remove(&oldest);
                }
            }
        }
        self.link_previews.insert(url, preview);
    }

    pub(super) fn hover_totoz(&mut self, name: String, anchor: Point) -> Vec<Task<Message>> {
        // The widget reports the text anchor in the scrollable content
        // coordinate space; convert it back to window coordinates so the
        // popup is anchored above the totoz and stays still while scrolling.
        let anchor = Point::new(anchor.x, anchor.y - self.scroll_offset);
        if let Some(image) = self.totoz.images.get(&name).cloned() {
            self.hovered_totoz = Some(HoveredTotoz {
                name,
                image: Some(image),
                anchor,
                frame_index: 0,
                frame_started: Instant::now(),
                loading: false,
                error: None,
            });
            return Vec::new();
        }
        self.hovered_totoz = Some(HoveredTotoz {
            name: name.clone(),
            image: None,
            anchor,
            frame_index: 0,
            frame_started: Instant::now(),
            loading: true,
            error: None,
        });
        let client = self.client.clone();
        let server = self.config.totoz_server.clone();
        vec![Task::perform(
            async move {
                let result = net::fetch_totoz_image(&client, &server, &name).await;
                (name, result)
            },
            |(name, result)| Message::TotozHoverImage(name, result),
        )]
    }
}
