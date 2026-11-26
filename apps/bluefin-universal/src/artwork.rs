//! Bounded artwork for the desktop canvas and legacy semantic browser.
use crate::protocol::{self, Result, Session};
use eframe::egui;
use futures_util::future::{AbortHandle, Abortable};
use std::{
    collections::{HashMap, VecDeque},
    sync::mpsc,
};
use uuid::Uuid;

type Key = (Uuid, bool);
const MAX_ENTRIES: usize = 96;
const MAX_PIXELS_BYTES: usize = 24 * 1024 * 1024;
struct Decoded {
    image: egui::ColorImage,
    #[cfg(target_arch = "wasm32")]
    bytes: Vec<u8>,
}
struct Asset {
    texture: Option<egui::TextureHandle>,
    used: u64,
    cost: usize,
    #[cfg(target_arch = "wasm32")]
    url: Option<String>,
}
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(
    inline_js = "export function bluefinArtUrl(bytes){return URL.createObjectURL(new Blob([bytes],{type:bytes[0]===137?'image/png':'image/jpeg'}));} export function bluefinDropArtUrl(url){URL.revokeObjectURL(url);} export function bluefinVisibleArt(){const root=document.getElementById('semantic');if(!root||root.hidden)return [];return Array.from(root.querySelectorAll('[data-artwork]')).filter(node=>{const rect=node.getBoundingClientRect();return rect.bottom>-200&&rect.top<innerHeight+200;}).slice(0,48).map(node=>node.dataset.artwork);}"
)]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_name=bluefinArtUrl)]
    fn art_url(bytes: &[u8]) -> String;
    #[wasm_bindgen::prelude::wasm_bindgen(js_name=bluefinDropArtUrl)]
    fn drop_url(url: &str);
    #[wasm_bindgen::prelude::wasm_bindgen(js_name=bluefinVisibleArt)]
    fn visible_art() -> js_sys::Array;
}
impl Drop for Asset {
    fn drop(&mut self) {
        #[cfg(target_arch = "wasm32")]
        if let Some(url) = &self.url {
            drop_url(url);
        }
    }
}
pub struct Cache {
    generation: u64,
    clock: u64,
    assets: HashMap<Key, Asset>,
    queue: VecDeque<Key>,
    active: HashMap<Key, AbortHandle>,
    tx: mpsc::Sender<(u64, Key, Result<Decoded>)>,
    rx: mpsc::Receiver<(u64, Key, Result<Decoded>)>,
}
impl Default for Cache {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            generation: 0,
            clock: 0,
            assets: HashMap::new(),
            queue: VecDeque::new(),
            active: HashMap::new(),
            tx,
            rx,
        }
    }
}
impl Cache {
    pub fn cancel_pending(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        for (_, handle) in self.active.drain() {
            handle.abort();
        }
        self.queue.clear();
        while self.rx.try_recv().is_ok() {}
    }
    pub fn clear(&mut self) {
        self.cancel_pending();
        self.assets.clear();
    }
    #[cfg(target_arch = "wasm32")]
    pub fn request_visible(&mut self) {
        for value in visible_art().iter().filter_map(|value| value.as_string()) {
            if let Some((id, backdrop)) = value.split_once(':')
                && let Ok(id) = Uuid::parse_str(id)
            {
                self.request(id, backdrop == "true");
            }
        }
    }
    pub fn request(&mut self, id: Uuid, backdrop: bool) {
        let key = (id, backdrop);
        self.clock = self.clock.wrapping_add(1);
        if let Some(asset) = self.assets.get_mut(&key) {
            asset.used = self.clock;
            return;
        }
        if !id.is_nil()
            && !self.active.contains_key(&key)
            && !self.queue.contains(&key)
            && self.queue.len() < 64
        {
            self.queue.push_back(key);
        }
    }
    pub fn texture(&self, id: Uuid, backdrop: bool) -> Option<egui::TextureHandle> {
        self.assets
            .get(&(id, backdrop))
            .and_then(|asset| asset.texture.clone())
    }
    #[cfg(target_arch = "wasm32")]
    pub fn url(&self, id: Uuid, backdrop: bool) -> Option<&str> {
        self.assets
            .get(&(id, backdrop))
            .and_then(|asset| asset.url.as_deref())
    }
    pub fn poll(&mut self, ctx: &egui::Context, session: Option<&Session>) {
        while let Ok((generation, key, result)) = self.rx.try_recv() {
            if generation != self.generation {
                continue;
            }
            self.active.remove(&key);
            self.clock = self.clock.wrapping_add(1);
            let mut asset = Asset {
                texture: None,
                used: self.clock,
                cost: 0,
                #[cfg(target_arch = "wasm32")]
                url: None,
            };
            if let Ok(decoded) = result {
                asset.cost = decoded.image.pixels.len() * 4;
                #[cfg(target_arch = "wasm32")]
                {
                    asset.cost += decoded.bytes.len();
                    asset.url = Some(art_url(&decoded.bytes));
                }
                asset.texture = Some(ctx.load_texture(
                    format!("art-{}-{}", key.0, key.1),
                    decoded.image,
                    egui::TextureOptions::LINEAR,
                ));
            }
            while self.assets.len() >= MAX_ENTRIES
                || self.assets.values().map(|a| a.cost).sum::<usize>() + asset.cost
                    > MAX_PIXELS_BYTES
            {
                let Some(oldest) = self
                    .assets
                    .iter()
                    .min_by_key(|(_, a)| a.used)
                    .map(|(key, _)| *key)
                else {
                    break;
                };
                self.assets.remove(&oldest);
            }
            self.assets.insert(key, asset);
        }
        let Some(session) = session else {
            return;
        };
        while self.active.len() < 2 {
            let Some(key) = self.queue.pop_front() else {
                break;
            };
            if self.assets.contains_key(&key) {
                continue;
            }
            let session = session.clone();
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            let generation = self.generation;
            let (handle, registration) = AbortHandle::new_pair();
            self.active.insert(key, handle);
            let task = async move {
                let work = async move {
                    let bytes = protocol::artwork(&session, key.0, key.1).await?;
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        tokio::task::spawn_blocking(move || decode(bytes, key.1))
                            .await
                            .map_err(|_| "Artwork decoder stopped".to_owned())?
                    }
                    #[cfg(target_arch = "wasm32")]
                    {
                        decode(bytes, key.1)
                    }
                };
                if let Ok(result) = Abortable::new(work, registration).await {
                    let _ = tx.send((generation, key, result));
                    ctx.request_repaint();
                }
            };
            #[cfg(not(target_arch = "wasm32"))]
            {
                static RT: std::sync::OnceLock<tokio::runtime::Runtime> =
                    std::sync::OnceLock::new();
                RT.get_or_init(|| {
                    tokio::runtime::Builder::new_multi_thread()
                        .worker_threads(2)
                        .enable_all()
                        .build()
                        .expect("Artwork runtime")
                })
                .spawn(task);
            }
            #[cfg(target_arch = "wasm32")]
            wasm_bindgen_futures::spawn_local(task);
        }
    }
}
impl Drop for Cache {
    fn drop(&mut self) {
        self.clear();
    }
}
fn decode(bytes: Vec<u8>, backdrop: bool) -> Result<Decoded> {
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("Artwork exceeds bounds".into());
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|_| "Artwork format unavailable")?;
    if !matches!(
        reader.format(),
        Some(image::ImageFormat::Jpeg | image::ImageFormat::Png)
    ) {
        return Err("Artwork format unavailable".into());
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|_| "Artwork could not be decoded within bounds")?;
    let decoded = if backdrop {
        decoded.thumbnail(1200, 675)
    } else {
        decoded.thumbnail(240, 360)
    }
    .into_rgba8();
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [decoded.width() as usize, decoded.height() as usize],
        decoded.as_raw(),
    );
    Ok(Decoded {
        image,
        #[cfg(target_arch = "wasm32")]
        bytes,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_oversized_or_unsupported_images_without_allocating_a_texture() {
        assert!(decode(vec![0; 2 * 1024 * 1024 + 1], false).is_err());
        assert!(decode(b"<svg></svg>".to_vec(), false).is_err());
    }
    #[test]
    fn account_clear_drops_requests_and_ignores_old_completion() {
        let mut cache = Cache::default();
        let key = (Uuid::from_u128(1), false);
        let old = cache.generation;
        cache.request(key.0, key.1);
        cache.clear();
        cache
            .tx
            .send((old, key, Err("old account".into())))
            .unwrap();
        cache.poll(&egui::Context::default(), None);
        assert!(cache.assets.is_empty());
        assert!(cache.queue.is_empty());
        assert!(cache.active.is_empty());
    }
}
