//! Image and SVG resource loading (PNG, JPEG, WebP, SVG via resvg) with GPU caching and async placeholders.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::mpsc;

use rustui_core::ImageSourceData;

use crate::display_list::ImageId;

/// Decoded RGBA8 pixel payload ready for GPU texture upload.
#[derive(Debug, Clone)]
pub struct DecodedImage {
    pub id: ImageId,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Asynchronous image fetch status.
#[derive(Debug, Clone)]
pub enum AsyncImageState {
    Loading,
    Ready(ImageId),
    Failed(String),
}

/// Caches decoded raster and SVG images and supports background decoding with placeholders.
pub struct ResourceLoader {
    images: HashMap<ImageId, DecodedImage>,
    async_states: HashMap<u64, AsyncImageState>,
    async_tx: mpsc::Sender<(u64, Result<DecodedImage, String>)>,
    async_rx: mpsc::Receiver<(u64, Result<DecodedImage, String>)>,
}

impl std::fmt::Debug for ResourceLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceLoader")
            .field("cached_images", &self.images.len())
            .finish()
    }
}

impl Default for ResourceLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl ResourceLoader {
    pub fn new() -> Self {
        let (async_tx, async_rx) = mpsc::channel();
        Self {
            images: HashMap::new(),
            async_states: HashMap::new(),
            async_tx,
            async_rx,
        }
    }

    pub fn get(&self, id: ImageId) -> Option<&DecodedImage> {
        self.images.get(&id)
    }

    pub fn insert_rgba(&mut self, id: ImageId, width: u32, height: u32, rgba: Vec<u8>) {
        self.images.insert(
            id,
            DecodedImage {
                id,
                width,
                height,
                rgba,
            },
        );
    }

    /// Ensures an [`ImageSourceData`] is decoded and cached, returning its [`ImageId`].
    pub fn resolve_source(&mut self, source: &ImageSourceData) -> Option<ImageId> {
        match source {
            ImageSourceData::Rgba {
                key,
                width,
                height,
                pixels,
            } => {
                if !self.images.contains_key(key) {
                    self.insert_rgba(*key, *width, *height, pixels.as_ref().clone());
                }
                Some(*key)
            }
            ImageSourceData::EncodedBytes { key, bytes, is_svg } => {
                if self.images.contains_key(key) {
                    return Some(*key);
                }
                let decoded = if *is_svg {
                    Self::decode_svg(*key, bytes)
                } else {
                    Self::decode_raster(*key, bytes)
                };
                match decoded {
                    Ok(img) => {
                        self.images.insert(*key, img);
                        Some(*key)
                    }
                    Err(_) => None,
                }
            }
            ImageSourceData::PendingPlaceholder => None,
        }
    }

    /// Decodes PNG, JPEG, or WebP bytes into RGBA8.
    pub fn decode_raster(id: ImageId, bytes: &[u8]) -> Result<DecodedImage, String> {
        let dyn_img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
        let rgba = dyn_img.to_rgba8();
        let (width, height) = rgba.dimensions();
        Ok(DecodedImage {
            id,
            width,
            height,
            rgba: rgba.into_raw(),
        })
    }

    /// Parses and rasterizes an SVG document using `resvg`.
    pub fn decode_svg(id: ImageId, svg_bytes: &[u8]) -> Result<DecodedImage, String> {
        let opt = resvg::usvg::Options::default();
        let tree = resvg::usvg::Tree::from_data(svg_bytes, &opt).map_err(|e| e.to_string())?;
        let size = tree.size().to_int_size();
        let width = size.width().max(1);
        let height = size.height().max(1);
        let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
            .ok_or_else(|| "failed to allocate SVG pixmap".to_owned())?;
        resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pixmap.as_mut());

        // Convert premultiplied RGBA to unpremultiplied RGBA8 for consistent blending.
        let mut rgba = pixmap.take();
        for px in rgba.chunks_exact_mut(4) {
            let a = px[3];
            if a > 0 && a < 255 {
                let af = f32::from(a) / 255.0;
                px[0] = ((f32::from(px[0]) / af).min(255.0)) as u8;
                px[1] = ((f32::from(px[1]) / af).min(255.0)) as u8;
                px[2] = ((f32::from(px[2]) / af).min(255.0)) as u8;
            }
        }
        Ok(DecodedImage {
            id,
            width,
            height,
            rgba,
        })
    }

    /// Starts async background loading from encoded bytes or file path, returning a tracking ID.
    pub fn load_async_bytes(&mut self, key_seed: &str, bytes: Vec<u8>, is_svg: bool) -> u64 {
        let id = hash_str(key_seed);
        if self.images.contains_key(&id) {
            self.async_states.insert(id, AsyncImageState::Ready(id));
            return id;
        }
        self.async_states.insert(id, AsyncImageState::Loading);
        let tx = self.async_tx.clone();
        std::thread::spawn(move || {
            let res = if is_svg {
                Self::decode_svg(id, &bytes)
            } else {
                Self::decode_raster(id, &bytes)
            };
            let _ = tx.send((id, res));
        });
        id
    }

    /// Polls completed async image decodes.
    pub fn poll_async(&mut self) -> usize {
        let mut completed = 0;
        while let Ok((id, res)) = self.async_rx.try_recv() {
            match res {
                Ok(img) => {
                    self.images.insert(id, img);
                    self.async_states.insert(id, AsyncImageState::Ready(id));
                }
                Err(e) => {
                    self.async_states.insert(id, AsyncImageState::Failed(e));
                }
            }
            completed += 1;
        }
        completed
    }

    pub fn async_state(&self, id: u64) -> Option<&AsyncImageState> {
        self.async_states.get(&id)
    }
}

fn hash_str(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Asset bundling helper embedding static asset bytes at compile time (`include_bytes!`).
#[macro_export]
macro_rules! include_asset {
    ($path:literal) => {
        include_bytes!($path)
    };
}
