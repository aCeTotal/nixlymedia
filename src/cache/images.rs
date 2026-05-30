use std::collections::HashMap;
use std::sync::Arc;

use egui::{ColorImage, Context, TextureHandle, TextureOptions};
use parking_lot::Mutex;
use tokio::runtime::Handle;

use crate::api::Api;

enum Slot {
    Loading,
    Ready(TextureHandle),
    Failed,
}

#[derive(Clone)]
pub struct ImageCache {
    inner: Arc<Mutex<Inner>>,
    api: Api,
    rt: Handle,
}

struct Inner {
    slots: HashMap<String, Slot>,
    pending: HashMap<String, ColorImage>,
    ctx: Option<Context>,
}

impl ImageCache {
    pub fn new(api: Api, rt: Handle) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                slots: HashMap::new(),
                pending: HashMap::new(),
                ctx: None,
            })),
            api,
            rt,
        }
    }

    pub fn pump(&self, ctx: &Context) {
        let mut g = self.inner.lock();
        if g.ctx.is_none() {
            g.ctx = Some(ctx.clone());
        }
        let ready: Vec<(String, ColorImage)> = g.pending.drain().collect();
        for (key, img) in ready {
            let handle = ctx.load_texture(&key, img, TextureOptions::LINEAR);
            g.slots.insert(key, Slot::Ready(handle));
        }
    }

    pub fn get(&self, name: &str) -> Option<TextureHandle> {
        let mut g = self.inner.lock();
        match g.slots.get(name) {
            Some(Slot::Ready(h)) => Some(h.clone()),
            Some(_) => None,
            None => {
                g.slots.insert(name.to_string(), Slot::Loading);
                drop(g);
                self.spawn_fetch(name.to_string());
                None
            }
        }
    }

    fn spawn_fetch(&self, name: String) {
        let api = self.api.clone();
        let inner = self.inner.clone();
        let key = name.clone();
        let is_url = name.starts_with("http://") || name.starts_with("https://");
        let basename = name.rsplit('/').next().unwrap_or(&name).to_string();
        self.rt.spawn(async move {
            let result = if is_url {
                api.get_bytes_url(&name).await
            } else {
                let path = format!("/image/{}", urlencoding::encode(&basename));
                api.get_bytes(&path).await
            };
            match result {
                Ok(bytes) => match image::load_from_memory(&bytes) {
                    Ok(dyn_img) => {
                        let rgba = dyn_img.to_rgba8();
                        let (w, h) = rgba.dimensions();
                        let img = ColorImage::from_rgba_unmultiplied(
                            [w as usize, h as usize],
                            rgba.as_raw(),
                        );
                        let mut g = inner.lock();
                        g.pending.insert(key, img);
                        if let Some(ctx) = &g.ctx {
                            ctx.request_repaint();
                        }
                    }
                    Err(e) => {
                        eprintln!("image decode failed {basename}: {e}");
                        inner.lock().slots.insert(key, Slot::Failed);
                    }
                },
                Err(e) => {
                    eprintln!("image fetch failed {basename}: {e}");
                    inner.lock().slots.insert(key, Slot::Failed);
                }
            }
        });
    }
}
