//! Images in extension panels (fyx:ui 0.3, design.md §12, plan 4.11): decode off the main thread
//! within the §12 limits, cache per extension, draw clipped to the node's box.
//!
//! - **Sources:** `asset` files under the extension's installed folder, and `generated` images
//!   from `images.add-image` (kept here, freed on `remove-image`, a reload or a stop).
//! - **Decoding** never runs on the main thread or in `render()`. A render notes the images it
//!   lacks ([`ImageCache::want`]); the view hands them over after the frame
//!   ([`ImageCache::request`]), and a background task decodes them. At most
//!   [`MAX_CONCURRENT_DECODES_PER_EXTENSION`] per extension and [`MAX_CONCURRENT_DECODES`] in all;
//!   one that takes longer than [`DECODE_TIMEOUT`] or panics is a failure. A decode past its
//!   timeout keeps its slot until it returns, so runaway decodes cannot pile up past the caps
//!   (integration-5 S3).
//! - **Bytes:** assets get the same allocation-free checks as generated images
//!   (`check_image_bytes`: no `.svgz`, no entity declarations, the PNG header) before decoding.
//! - **PNG:** the header is read first; anything over [`MAX_IMAGE_SIDE`] on a side is refused
//!   before a pixel is allocated, and the decoder's allocation is capped.
//! - **SVG:** first a scan of the XML ([`prescan_svg`]): every element counts with what its
//!   references (`href="#…"`, `url(#…)`, in attributes and in `<style>`) would expand to, so a
//!   `<use>` bomb is refused before `usvg` expands it. Then `usvg` with every `<image>` href
//!   refused, no resource folder, and only Fyx's bundled fonts (loaded once, on the first SVG;
//!   the system's are never read, so they cannot be probed), then the node and blur caps on the
//!   parsed tree, then `resvg` at the box's pixel size.
//! - **Cache:** per extension, by (source, pixel size, tint), least recently drawn first out once
//!   [`MAX_DECODED_BYTES_PER_EXTENSION`] is reached. An image larger than what is left shows its
//!   alt text, and the log says "image limit reached".
//! - Until an image is ready, or when it failed, the node draws its alt text in a muted box (with a
//!   warning icon for a failure).

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context as _, Result, anyhow, bail};
use futures::FutureExt as _;
use gpui::{App, AppContext as _, Context, Entity, Global, RenderImage};
use ui_ext::image::{
    DECODE_TIMEOUT, ImageFormat, ImageSource, MAX_ASSET_BYTES, MAX_CONCURRENT_DECODES,
    MAX_CONCURRENT_DECODES_PER_EXTENSION, MAX_DECODED_BYTES_PER_EXTENSION, MAX_DECODED_IMAGE_BYTES,
    MAX_IMAGE_SIDE, MAX_SVG_BLUR_STD_DEVIATION, MAX_SVG_NODES, MAX_SVG_USE_DEPTH,
};

/// One decoded image: the source, its pixel size and the tint (as 0xRRGGBBAA) it was drawn with.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ImageKey {
    pub source: SourceKey,
    /// Pixels; `None` for a PNG, decoded at its own size and scaled when drawn.
    pub pixels: Option<(u32, u32)>,
    pub tint: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SourceKey {
    Asset(String),
    /// A handle and the generation of its bytes, so a handle reused after `remove-image` never
    /// shows the old picture.
    Generated(u32, u64),
}

/// What a node draws now.
#[derive(Clone)]
pub enum ImageState {
    Ready(Arc<RenderImage>),
    Pending,
    Failed,
}

struct Decoded {
    image: Arc<RenderImage>,
    bytes: u64,
    last_used: Cell<u64>,
}

#[derive(Default)]
struct ExtensionImages {
    /// The installed version folder, where `assets/` lives.
    dir: Option<PathBuf>,
    generated: HashMap<u32, (u64, ImageFormat, Arc<[u8]>)>,
    next_generation: u64,
    decoded: HashMap<ImageKey, Decoded>,
    used_bytes: u64,
    failed: HashSet<ImageKey>,
    pending: HashSet<ImageKey>,
    queue: VecDeque<ImageKey>,
    running: usize,
}

impl ExtensionImages {
    fn evict_for(&mut self, bytes: u64) -> bool {
        while self.used_bytes + bytes > MAX_DECODED_BYTES_PER_EXTENSION {
            let Some(oldest) = self
                .decoded
                .iter()
                .min_by_key(|(_, decoded)| decoded.last_used.get())
                .map(|(key, _)| key.clone())
            else {
                return false;
            };
            if let Some(evicted) = self.decoded.remove(&oldest) {
                self.used_bytes -= evicted.bytes;
            }
        }
        true
    }
}

/// The decoded images of every extension (global).
#[derive(Default)]
pub struct ImageCache {
    extensions: HashMap<Arc<str>, ExtensionImages>,
    running: usize,
    clock: Cell<u64>,
    /// Images renders found missing, for [`Self::request`] after the frame.
    wanted: RefCell<Vec<(Arc<str>, ImageKey)>>,
    /// Fyx's bundled fonts for SVG text, loaded on the first SVG decode.
    fonts: Option<Arc<usvg::fontdb::Database>>,
}

/// A font database with only the fonts bundled in Fyx's assets (`fonts/**/*.ttf`).
fn bundled_fonts(cx: &App) -> Arc<usvg::fontdb::Database> {
    let mut fonts = usvg::fontdb::Database::new();
    let source = cx.asset_source();
    for path in source.list("fonts").unwrap_or_default() {
        if path.ends_with(".ttf")
            && let Ok(Some(bytes)) = source.load(&path)
        {
            fonts.load_font_data(bytes.into_owned());
        }
    }
    fonts.set_sans_serif_family("IBM Plex Sans");
    fonts.set_serif_family("IBM Plex Sans");
    fonts.set_monospace_family("Lilex");
    Arc::new(fonts)
}

struct GlobalImageCache(Entity<ImageCache>);

impl Global for GlobalImageCache {}

pub(crate) fn init(cx: &mut App) {
    let cache = cx.new(|_| ImageCache::default());
    cx.set_global(GlobalImageCache(cache));
}

impl ImageCache {
    pub fn global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalImageCache>()
            .map(|global| global.0.clone())
    }

    /// Where an extension's assets are (its installed version folder).
    pub fn set_dir(&mut self, extension_id: &str, dir: PathBuf) {
        self.extensions.entry(extension_id.into()).or_default().dir = Some(dir);
    }

    /// `add-image`.
    pub fn add_generated(
        &mut self,
        extension_id: &str,
        handle: u32,
        format: ImageFormat,
        bytes: Arc<[u8]>,
    ) {
        let images = self.extensions.entry(extension_id.into()).or_default();
        images.next_generation += 1;
        let generation = images.next_generation;
        images.generated.insert(handle, (generation, format, bytes));
    }

    /// `remove-image`: its decoded copies go too; nodes that still name it show their alt text.
    pub fn remove_generated(&mut self, extension_id: &str, handle: u32, cx: &mut Context<Self>) {
        let Some(images) = self.extensions.get_mut(extension_id) else {
            return;
        };
        if let Some((generation, _, _)) = images.generated.remove(&handle) {
            let gone = SourceKey::Generated(handle, generation);
            let keys: Vec<ImageKey> = images
                .decoded
                .keys()
                .filter(|key| key.source == gone)
                .cloned()
                .collect();
            for key in keys {
                if let Some(decoded) = images.decoded.remove(&key) {
                    images.used_bytes -= decoded.bytes;
                }
            }
            cx.notify();
        }
    }

    /// A reload or a stop: every generated image and decoded copy goes; the folder stays.
    pub fn clear(&mut self, extension_id: &str, cx: &mut Context<Self>) {
        if let Some(images) = self.extensions.get_mut(extension_id) {
            let dir = images.dir.take();
            let running = images.running;
            *images = ExtensionImages {
                dir,
                running,
                ..ExtensionImages::default()
            };
            cx.notify();
        }
    }

    /// The decoded bytes an extension holds now.
    pub fn used_bytes(&self, extension_id: &str) -> u64 {
        self.extensions
            .get(extension_id)
            .map_or(0, |images| images.used_bytes)
    }

    /// The key a node needs: `pixels` is the node's box in device pixels (an SVG is drawn at that
    /// size; a PNG is decoded at its own). `None` when the generated handle is not live.
    pub fn key_for(
        &self,
        extension_id: &str,
        source: &ImageSource,
        format_hint: Option<ImageFormat>,
        pixels: (u32, u32),
        tint: Option<u32>,
    ) -> Option<ImageKey> {
        let (source, format) = match source {
            ImageSource::Asset(path) => (
                SourceKey::Asset(path.clone()),
                format_hint.or_else(|| ui_ext::image::check_asset_path(path).ok())?,
            ),
            ImageSource::Generated(handle) => {
                let (generation, format, _) =
                    self.extensions.get(extension_id)?.generated.get(handle)?;
                (SourceKey::Generated(*handle, *generation), *format)
            }
        };
        let pixels = match format {
            ImageFormat::Png => None,
            ImageFormat::Svg => Some((
                pixels.0.clamp(1, MAX_IMAGE_SIDE),
                pixels.1.clamp(1, MAX_IMAGE_SIDE),
            )),
        };
        Some(ImageKey {
            source,
            pixels,
            // A PNG takes no tint (§12: ignored, logged at decode).
            tint: if format == ImageFormat::Svg {
                tint
            } else {
                None
            },
        })
    }

    /// What to draw for `key` now. A miss is noted for [`Self::request`].
    pub fn get(&self, extension_id: &str, key: &ImageKey) -> ImageState {
        let Some(images) = self.extensions.get(extension_id) else {
            self.want(extension_id, key);
            return ImageState::Pending;
        };
        if let Some(decoded) = images.decoded.get(key) {
            let now = self.clock.get() + 1;
            self.clock.set(now);
            decoded.last_used.set(now);
            return ImageState::Ready(decoded.image.clone());
        }
        if images.failed.contains(key) {
            return ImageState::Failed;
        }
        if !images.pending.contains(key) {
            self.want(extension_id, key);
        }
        ImageState::Pending
    }

    fn want(&self, extension_id: &str, key: &ImageKey) {
        self.wanted
            .borrow_mut()
            .push((extension_id.into(), key.clone()));
    }

    /// Whether renders noted images to request.
    pub fn has_wanted(&self) -> bool {
        !self.wanted.borrow().is_empty()
    }

    /// Queues the decodes renders asked for and starts what the limits allow.
    pub fn request_wanted(&mut self, cx: &mut Context<Self>) {
        let wanted = std::mem::take(&mut *self.wanted.borrow_mut());
        for (extension_id, key) in wanted {
            self.request(&extension_id, key, cx);
        }
    }

    pub fn request(&mut self, extension_id: &str, key: ImageKey, cx: &mut Context<Self>) {
        let images = self.extensions.entry(extension_id.into()).or_default();
        if images.decoded.contains_key(&key)
            || images.failed.contains(&key)
            || !images.pending.insert(key.clone())
        {
            return;
        }
        images.queue.push_back(key);
        self.pump(cx);
    }

    fn pump(&mut self, cx: &mut Context<Self>) {
        while self.running < MAX_CONCURRENT_DECODES {
            let next = self.extensions.iter_mut().find_map(|(id, images)| {
                (images.running < MAX_CONCURRENT_DECODES_PER_EXTENSION)
                    .then(|| images.queue.pop_front().map(|key| (id.clone(), key)))
                    .flatten()
            });
            let Some((extension_id, key)) = next else {
                return;
            };
            if self.fonts.is_none() {
                self.fonts = Some(bundled_fonts(cx));
            }
            let Some(job) = self.job(&extension_id, &key) else {
                self.finish(&extension_id, key, Err(anyhow!("its source is gone")), cx);
                continue;
            };
            self.running += 1;
            if let Some(images) = self.extensions.get_mut(&extension_id) {
                images.running += 1;
            }
            let decode = cx.background_spawn(async move { decode_job(job) });
            let timer = cx.background_executor().timer(DECODE_TIMEOUT);
            cx.spawn(async move |this, cx| {
                let mut decode = decode.fuse();
                let mut timer = timer.fuse();
                let done = futures::select_biased! {
                    result = decode => Some(result),
                    _ = timer => None,
                };
                let release = |this: &mut Self, cx: &mut Context<Self>| {
                    this.running -= 1;
                    if let Some(images) = this.extensions.get_mut(&extension_id) {
                        images.running = images.running.saturating_sub(1);
                    }
                    this.pump(cx);
                };
                match done {
                    Some(result) => {
                        this.update(cx, |this, cx| {
                            this.finish(&extension_id, key, result, cx);
                            release(this, cx);
                        })
                        .ok();
                    }
                    None => {
                        // The image fails now; its thread cannot be stopped, so the slot stays
                        // taken until the decode returns (integration-5 S3).
                        let timeout = anyhow!("decoding took longer than {DECODE_TIMEOUT:?}");
                        this.update(cx, |this, cx| {
                            this.finish(&extension_id, key, Err(timeout), cx)
                        })
                        .ok();
                        decode.await.ok();
                        this.update(cx, |this, cx| release(this, cx)).ok();
                    }
                }
            })
            .detach();
        }
    }

    fn job(&self, extension_id: &str, key: &ImageKey) -> Option<DecodeJob> {
        let images = self.extensions.get(extension_id)?;
        let input = match &key.source {
            SourceKey::Asset(path) => DecodeInput::Asset {
                dir: images.dir.clone()?,
                path: path.clone(),
            },
            SourceKey::Generated(handle, generation) => {
                let (live, format, bytes) = images.generated.get(handle)?;
                if live != generation {
                    return None;
                }
                DecodeInput::Bytes(*format, bytes.clone())
            }
        };
        Some(DecodeJob {
            input,
            pixels: key.pixels,
            tint: key.tint,
            fonts: self.fonts.clone().unwrap_or_default(),
        })
    }

    fn finish(
        &mut self,
        extension_id: &str,
        key: ImageKey,
        result: Result<(RenderImage, u64)>,
        cx: &mut Context<Self>,
    ) {
        let Some(images) = self.extensions.get_mut(extension_id) else {
            return;
        };
        if !images.pending.remove(&key) {
            // Cleared (a reload, a stop) while it decoded.
            return;
        }
        match result {
            Ok((image, bytes)) => {
                if bytes > MAX_DECODED_BYTES_PER_EXTENSION || !images.evict_for(bytes) {
                    log::warn!(
                        "fyx:ui: {extension_id}: image limit reached for {:?}",
                        key.source
                    );
                    images.failed.insert(key);
                } else {
                    images.used_bytes += bytes;
                    images.decoded.insert(
                        key,
                        Decoded {
                            image: Arc::new(image),
                            bytes,
                            last_used: Cell::new(self.clock.get()),
                        },
                    );
                }
            }
            Err(error) => {
                log::warn!(
                    "fyx:ui: {extension_id}: image {:?} not shown: {}",
                    key.source,
                    ui_ext::sanitize_for_log(&format!("{error:#}"))
                );
                images.failed.insert(key);
            }
        }
        cx.notify();
    }
}

enum DecodeInput {
    Asset { dir: PathBuf, path: String },
    Bytes(ImageFormat, Arc<[u8]>),
}

struct DecodeJob {
    input: DecodeInput,
    pixels: Option<(u32, u32)>,
    tint: Option<u32>,
    fonts: Arc<usvg::fontdb::Database>,
}

fn decode_job(job: DecodeJob) -> Result<(RenderImage, u64)> {
    let (format, bytes) = match job.input {
        DecodeInput::Bytes(format, bytes) => (format, bytes),
        DecodeInput::Asset { dir, path } => {
            let format =
                ui_ext::image::check_asset_path(&path).map_err(|error| anyhow!("{error}"))?;
            (format, read_asset(&dir, &path)?.into())
        }
    };
    // The allocation-free checks, for assets as for generated bytes (integration-5 S3): no
    // `.svgz`, no entity declarations, a PNG header within the caps.
    ui_ext::image::check_image_bytes(format, &bytes, MAX_ASSET_BYTES)
        .map_err(|error| anyhow!("{error}"))?;
    // A decoder panic is a host bug; it fails this image only (§12, threat 1).
    // The inputs are owned and dropped with the closure, so nothing half-updated is seen after a
    // panic.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || match format {
        ImageFormat::Png => decode_png(&bytes),
        ImageFormat::Svg => {
            let (width, height) = job.pixels.context("an SVG needs a size to be drawn at")?;
            decode_svg(&bytes, width, height, job.tint, job.fonts)
        }
    }))
    .map_err(|_| anyhow!("the decoder failed"))?
}

/// An asset of the installed folder: inside it after resolving links, at most 2 MiB.
pub(crate) fn read_asset(dir: &Path, path: &str) -> Result<Vec<u8>> {
    let root = std::fs::canonicalize(dir).with_context(|| format!("{}", dir.display()))?;
    let file =
        std::fs::canonicalize(root.join(path)).with_context(|| format!("{path} is missing"))?;
    if !file.starts_with(&root) {
        bail!("{path} resolves outside the extension's folder");
    }
    let size = std::fs::metadata(&file)?.len();
    if size > MAX_ASSET_BYTES as u64 {
        bail!("{path} is {size} bytes, the cap is {MAX_ASSET_BYTES}");
    }
    Ok(std::fs::read(&file)?)
}

/// A PNG (the first frame of an APNG) as BGRA. The header is checked against [`MAX_IMAGE_SIDE`]
/// before anything is allocated, and the decoder may allocate at most one full-size image.
pub fn decode_png(bytes: &[u8]) -> Result<(RenderImage, u64)> {
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(MAX_DECODED_IMAGE_BYTES * 2);
    reader.limits(limits);
    let mut rgba = reader.decode().context("decoding the PNG")?.into_rgba8();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let bytes = rgba.len() as u64;
    let frame = image::Frame::new(rgba);
    Ok((RenderImage::new(vec![frame]), bytes))
}

/// An SVG drawn at `width` × `height` pixels, as BGRA. No `<image>` href is resolved and no file
/// is read; text uses `fonts` only (Fyx's bundled fonts); the parsed tree must stay under the node
/// and blur caps.
pub fn decode_svg(
    bytes: &[u8],
    width: u32,
    height: u32,
    tint: Option<u32>,
    fonts: Arc<usvg::fontdb::Database>,
) -> Result<(RenderImage, u64)> {
    if width == 0 || height == 0 || width > MAX_IMAGE_SIDE || height > MAX_IMAGE_SIDE {
        bail!(
            "an SVG is drawn at most {MAX_IMAGE_SIDE} x {MAX_IMAGE_SIDE}, not {width} x {height}"
        );
    }
    let options = usvg::Options {
        resources_dir: None,
        fontdb: fonts,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    let text = std::str::from_utf8(bytes).context("an SVG is UTF-8 text")?;
    prescan_svg(text)?;
    let tree = usvg::Tree::from_data(bytes, &options).context("parsing the SVG")?;
    check_svg_tree(tree.root())?;
    let size = tree.size();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).context("allocating")?;
    let transform = resvg::tiny_skia::Transform::from_scale(
        width as f32 / size.width(),
        height as f32 / size.height(),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let mut data = pixmap.take();
    for pixel in data.chunks_exact_mut(4) {
        match tint {
            // A one-colour mask, like host icons: the tint at the drawing's coverage.
            Some(rgba) => {
                let coverage = pixel[3] as u32;
                let [r, g, b, a] = rgba.to_be_bytes();
                let alpha = coverage * a as u32 / 255;
                pixel.copy_from_slice(&[b, g, r, alpha as u8]);
            }
            None => gpui::swap_rgba_pa_to_bgra(pixel),
        }
    }
    let bytes = data.len() as u64;
    let buffer = image::RgbaImage::from_raw(width, height, data).context("the drawn SVG")?;
    let frame = image::Frame::new(buffer);
    Ok((RenderImage::new(vec![frame]), bytes))
}

/// Elements nested deeper than this are refused by the scan (the expansion recurses per level).
const MAX_SVG_NESTING: usize = 256;

/// The scan before `usvg` parses an SVG (integration-5 S3): `usvg` expands `<use>` (and
/// references to patterns, markers, clip paths) while it parses, before [`check_svg_tree`] can
/// count anything, so ten levels of ten `<use>`s would make 10^10 nodes. This counts every element
/// with what its references would expand to, without building anything:
/// - an element is 1 plus its children plus, for each reference it makes (`href="#id"` in any
///   namespace, `url(#id)` in any attribute, `style` included), what the referenced element
///   expands to;
/// - a `<style>` element may not use `url(…)`: its selectors apply references to elements this
///   scan cannot see, so they could not be counted;
/// - the whole must stay within [`MAX_SVG_NODES`], chains of references within
///   [`MAX_SVG_USE_DEPTH`], nesting within [`MAX_SVG_NESTING`]; a reference cycle is refused.
///
/// It over-counts rather than under-counts: a reference `usvg` would ignore still counts.
pub(crate) fn prescan_svg(text: &str) -> Result<()> {
    #[derive(Default)]
    struct Element {
        children: Vec<usize>,
        references: Vec<String>,
    }
    let mut elements: Vec<Element> = Vec::new();
    let mut roots = Vec::new();
    let mut ids: HashMap<String, usize> = HashMap::new();
    // Open elements, and whether each is a `<style>`.
    let mut open: Vec<(usize, bool)> = Vec::new();
    let mut at = 0;
    let check_text = |text: &str, open: &[(usize, bool)]| -> Result<()> {
        if open.last().is_some_and(|(_, style)| *style) && decode_references(text).contains("url(")
        {
            bail!("a <style> in an SVG may not use url(…); use the element's fill or style");
        }
        Ok(())
    };
    while let Some(offset) = text[at..].find('<') {
        let start = at + offset;
        check_text(&text[at..start], &open)?;
        let rest = &text[start..];
        let skip_to = |end: &str| rest.find(end).map_or(rest.len(), |found| found + end.len());
        if rest.starts_with("<!--") {
            at = start + skip_to("-->");
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            let end = skip_to("]]>");
            check_text(&rest[..end], &open)?;
            at = start + end;
            continue;
        }
        if rest.starts_with("<?") {
            at = start + skip_to("?>");
            continue;
        }
        if rest.starts_with("<!") {
            at = start + skip_to(">");
            continue;
        }
        if rest.starts_with("</") {
            open.pop();
            at = start + skip_to(">");
            continue;
        }
        let end = tag_end(rest).context("an SVG tag is not closed")?;
        let tag = &rest[1..end];
        let self_closing = tag.ends_with('/');
        let name = tag
            .split(|character: char| character.is_whitespace() || character == '/')
            .next()
            .unwrap_or_default();
        let is_style = name.rsplit(':').next() == Some("style");
        let index = elements.len();
        if index >= MAX_SVG_NODES {
            bail!("the SVG has more than {MAX_SVG_NODES} nodes");
        }
        let mut element = Element::default();
        for (name, value) in attributes(tag.trim_end_matches('/')) {
            let value = decode_references(value);
            if name == "id" {
                ids.insert(value.clone(), index);
            }
            if (name == "href" || name.ends_with(":href"))
                && let Some(id) = value.trim().strip_prefix('#')
            {
                element.references.push(id.to_string());
            }
            element.references.extend(url_references(&value));
        }
        elements.push(element);
        match open.last() {
            Some(&(parent, _)) => elements[parent].children.push(index),
            None => roots.push(index),
        }
        if !self_closing {
            open.push((index, is_style));
            if open.len() > MAX_SVG_NESTING {
                bail!("the SVG nests more than {MAX_SVG_NESTING} elements deep");
            }
        }
        at = start + end + 1;
    }

    /// (nodes it expands to, longest chain of references below it)
    fn expand(
        index: usize,
        elements: &[Element],
        ids: &HashMap<String, usize>,
        memo: &mut [Option<(usize, usize)>],
        visiting: &mut [bool],
    ) -> Result<(usize, usize)> {
        if let Some(known) = memo[index] {
            return Ok(known);
        }
        if visiting[index] {
            bail!("the SVG has a reference cycle");
        }
        visiting[index] = true;
        let mut nodes = 1usize;
        let mut chain = 0;
        for &child in &elements[index].children {
            let (child_nodes, child_chain) = expand(child, elements, ids, memo, visiting)?;
            nodes = nodes.saturating_add(child_nodes);
            chain = chain.max(child_chain);
            if nodes > MAX_SVG_NODES {
                bail!("the SVG has more than {MAX_SVG_NODES} nodes once its references expand");
            }
        }
        for reference in &elements[index].references {
            let Some(&target) = ids.get(reference) else {
                continue;
            };
            let (target_nodes, target_chain) = expand(target, elements, ids, memo, visiting)?;
            nodes = nodes.saturating_add(target_nodes);
            chain = chain.max(target_chain + 1);
            if chain > MAX_SVG_USE_DEPTH {
                bail!("the SVG nests references more than {MAX_SVG_USE_DEPTH} deep");
            }
            if nodes > MAX_SVG_NODES {
                bail!("the SVG has more than {MAX_SVG_NODES} nodes once its references expand");
            }
        }
        visiting[index] = false;
        memo[index] = Some((nodes, chain));
        Ok((nodes, chain))
    }
    let mut memo = vec![None; elements.len()];
    let mut visiting = vec![false; elements.len()];
    let mut total = 0usize;
    for root in roots {
        let (nodes, _) = expand(root, &elements, &ids, &mut memo, &mut visiting)?;
        total = total.saturating_add(nodes);
        if total > MAX_SVG_NODES {
            bail!("the SVG has more than {MAX_SVG_NODES} nodes once its references expand");
        }
    }
    Ok(())
}

/// Where a start tag ends: the first `>` outside a quoted attribute value.
fn tag_end(tag: &str) -> Option<usize> {
    let mut quote = None;
    for (index, character) in tag.char_indices() {
        match (quote, character) {
            (None, '"' | '\'') => quote = Some(character),
            (Some(open), _) if character == open => quote = None,
            (None, '>') => return Some(index),
            _ => {}
        }
    }
    None
}

/// A start tag's attributes (the text between `<` and `>`, the name first), as written.
fn attributes(tag: &str) -> Vec<(&str, &str)> {
    let mut found = Vec::new();
    let mut rest = tag
        .trim_start()
        .split_once(|character: char| character.is_whitespace())
        .map_or("", |(_, attributes)| attributes);
    loop {
        rest = rest.trim_start();
        let Some(equals) = rest.find('=') else {
            break;
        };
        let name = rest[..equals].trim();
        let value_start = rest[equals + 1..].trim_start();
        let Some(quote) = value_start
            .chars()
            .next()
            .filter(|c| *c == '"' || *c == '\'')
        else {
            break;
        };
        let value_text = &value_start[1..];
        let Some(close) = value_text.find(quote) else {
            break;
        };
        found.push((name, &value_text[..close]));
        rest = &value_text[close + 1..];
    }
    found
}

/// Character references decoded the way the XML parser will, so `&#35;a` is the reference `#a`.
fn decode_references(value: &str) -> String {
    if !value.contains('&') {
        return value.to_string();
    }
    let mut decoded = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(amp) = rest.find('&') {
        decoded.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let Some(semicolon) = after.find(';').filter(|end| *end <= 10) else {
            decoded.push('&');
            rest = after;
            continue;
        };
        let name = &after[..semicolon];
        let character = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match character {
            Some(character) => {
                decoded.push(character);
                rest = &after[semicolon + 1..];
            }
            None => {
                decoded.push('&');
                rest = after;
            }
        }
    }
    decoded.push_str(rest);
    decoded
}

/// The ids of every `url(#id)` in `text` (quotes and spaces allowed, as CSS allows them).
fn url_references(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("url(") {
        let after = &rest[start + 4..];
        let Some(end) = after.find(')') else {
            break;
        };
        let inner = after[..end]
            .trim()
            .trim_matches(|c| c == '"' || c == '\'')
            .trim();
        if let Some(id) = inner.strip_prefix('#') {
            found.push(id.to_string());
        }
        rest = &after[end..];
    }
    found
}

/// §12: at most [`MAX_SVG_NODES`] nodes (nested `<use>` and patterns count where they are drawn),
/// and blurs of at most [`MAX_SVG_BLUR_STD_DEVIATION`].
fn check_svg_tree(root: &usvg::Group) -> Result<()> {
    fn walk(group: &usvg::Group, nodes: &mut usize) -> Result<()> {
        for filter in group.filters() {
            for primitive in filter.primitives() {
                if let usvg::filter::Kind::GaussianBlur(blur) = primitive.kind() {
                    let deviation = blur.std_dev_x().get().max(blur.std_dev_y().get());
                    if deviation > MAX_SVG_BLUR_STD_DEVIATION {
                        bail!(
                            "a blur of {deviation} is over the cap of {MAX_SVG_BLUR_STD_DEVIATION}"
                        );
                    }
                }
            }
        }
        for node in group.children() {
            *nodes += 1;
            if *nodes > MAX_SVG_NODES {
                bail!("the SVG has more than {MAX_SVG_NODES} nodes");
            }
            let mut result = Ok(());
            node.subroots(|subroot| {
                if result.is_ok() {
                    result = walk(subroot, nodes);
                }
            });
            result?;
            if let usvg::Node::Group(child) = node {
                walk(child, nodes)?;
            }
        }
        Ok(())
    }
    let mut nodes = 0;
    walk(root, &mut nodes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use std::time::Duration;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]));
        image
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    /// A PNG whose header claims `width` × `height`, with no pixel data: a decoder bomb.
    fn png_header(width: u32, height: u32) -> Vec<u8> {
        fn crc(data: &[u8]) -> u32 {
            let mut crc = 0xffff_ffffu32;
            for &byte in data {
                crc ^= byte as u32;
                for _ in 0..8 {
                    crc = if crc & 1 != 0 {
                        (crc >> 1) ^ 0xedb8_8320
                    } else {
                        crc >> 1
                    };
                }
            }
            !crc
        }
        let mut chunk = b"IHDR".to_vec();
        chunk.extend_from_slice(&width.to_be_bytes());
        chunk.extend_from_slice(&height.to_be_bytes());
        chunk.extend_from_slice(&[8, 6, 0, 0, 0]);
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(&chunk);
        bytes.extend_from_slice(&crc(&chunk).to_be_bytes());
        bytes
    }

    fn pixel(image: &RenderImage, x: u32, y: u32) -> [u8; 4] {
        let width = image.size(0).width.0 as u32;
        let bytes = image.as_bytes(0).expect("one frame");
        let at = ((y * width + x) * 4) as usize;
        [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]
    }

    #[test]
    fn test_png_decodes_to_bgra() {
        let (image, bytes) = decode_png(&png(4, 3)).expect("decodes");
        assert_eq!(bytes, 4 * 3 * 4);
        assert_eq!(pixel(&image, 0, 0), [30, 20, 10, 255], "BGRA");
    }

    /// §12 threat 1: the size in the header is refused before a pixel is allocated.
    #[test]
    fn test_png_bomb_is_refused_from_its_header() {
        let started = std::time::Instant::now();
        let error = decode_png(&png_header(100_000, 100_000)).expect_err("too large");
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "no large allocation"
        );
        assert!(
            format!("{error:#}").to_lowercase().contains("limit"),
            "{error:#}"
        );
        assert!(decode_png(&png_header(2049, 16)).is_err());
    }

    #[test]
    fn test_svg_draws_at_the_box_size_without_outside_references() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10">
            <rect width="10" height="10" fill="#ff0000"/>
            <image href="file:///C:/Windows/win.ini" width="10" height="10"/>
            <image xlink:href="https://example.com/track.png" width="10" height="10"/>
        </svg>"##;
        let (image, bytes) = decode_svg(svg, 20, 20, None, Default::default()).expect("draws");
        assert_eq!(bytes, 20 * 20 * 4);
        assert_eq!(
            pixel(&image, 10, 10),
            [0, 0, 255, 255],
            "red in BGRA, the hrefs dropped"
        );
        // Tinted: a one-colour mask.
        let (tinted, _) =
            decode_svg(svg, 20, 20, Some(0x00ff00ff), Default::default()).expect("draws");
        assert_eq!(pixel(&tinted, 10, 10), [0, 255, 0, 255]);
        assert!(decode_svg(svg, 0, 20, None, Default::default()).is_err());
        assert!(decode_svg(svg, 4096, 20, None, Default::default()).is_err());
    }

    /// SVG text uses Fyx's bundled fonts and nothing else: with none loaded it draws nothing (no
    /// system font is reached).
    #[test]
    fn test_svg_text_uses_bundled_fonts_only() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20">
            <text x="2" y="16" font-family="monospace" font-size="16" fill="black">Wi</text></svg>"#;
        let inked = |image: &RenderImage| {
            image
                .as_bytes(0)
                .expect("frame")
                .chunks_exact(4)
                .filter(|pixel| pixel[3] > 0)
                .count()
        };
        let (none, _) = decode_svg(svg, 40, 20, None, Default::default()).expect("draws");
        assert_eq!(inked(&none), 0, "no fonts, no text");
        let lilex = std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../assets/fonts/lilex/Lilex-Regular.ttf"),
        )
        .expect("bundled font");
        let mut fonts = usvg::fontdb::Database::new();
        fonts.load_font_data(lilex);
        fonts.set_monospace_family("Lilex");
        let (text, _) = decode_svg(svg, 40, 20, None, Arc::new(fonts)).expect("draws");
        assert!(inked(&text) > 10, "the bundled font draws the text");
    }

    #[test]
    fn test_svg_caps() {
        let mut many =
            String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">"#);
        for _ in 0..=MAX_SVG_NODES {
            many.push_str(r#"<rect width="1" height="1"/>"#);
        }
        many.push_str("</svg>");
        let error = decode_svg(many.as_bytes(), 10, 10, None, Default::default())
            .expect_err("too many nodes");
        assert!(format!("{error:#}").contains("nodes"), "{error:#}");

        let blur = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
            <filter id="b"><feGaussianBlur stdDeviation="100"/></filter>
            <rect width="10" height="10" filter="url(#b)"/></svg>"#;
        let error =
            decode_svg(blur, 10, 10, None, Default::default()).expect_err("blur over the cap");
        assert!(format!("{error:#}").contains("blur"), "{error:#}");
    }

    /// integration-5 S3: a `<use>` bomb is refused by the scan, before `usvg` expands it, also
    /// when its references hide in a character reference or a `<style>`; cycles and deep
    /// nesting too. An ordinary drawing with references passes.
    #[test]
    fn test_svg_scan_refuses_reference_bombs() {
        // Ten levels of ten uses: 10^10 nodes once expanded, 100 as written.
        let mut bomb = String::from(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><defs><rect id="l0" width="1" height="1"/>"#,
        );
        for level in 1..=10 {
            bomb.push_str(&format!(r#"<g id="l{level}">"#));
            for _ in 0..10 {
                bomb.push_str(&format!(r##"<use xlink:href="#l{}"/>"##, level - 1));
            }
            bomb.push_str("</g>");
        }
        bomb.push_str(r##"</defs><use href="#l10"/></svg>"##);
        let started = std::time::Instant::now();
        let error =
            decode_svg(bomb.as_bytes(), 10, 10, None, Default::default()).expect_err("a use bomb");
        assert!(format!("{error:#}").contains("expand"), "{error:#}");
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "refused before expanding"
        );

        let hidden = bomb.replace(r##"xlink:href="#"##, r##"xlink:href="&#35;"##);
        assert!(
            prescan_svg(&hidden).is_err(),
            "a character reference does not hide it"
        );

        let mut styled = String::from(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><defs><pattern id="p0" width="1" height="1"><rect width="1" height="1"/></pattern>"#,
        );
        for level in 1..=8 {
            styled.push_str(&format!(r#"<pattern id="p{level}" width="1" height="1">"#));
            for index in 0..10 {
                styled.push_str(&format!(
                    r#"<rect class="c{level}x{index}" width="1" height="1"/>"#
                ));
            }
            styled.push_str("</pattern>");
        }
        styled.push_str("<style>");
        for level in 1..=8 {
            for index in 0..10 {
                styled.push_str(&format!(".c{level}x{index}{{fill:url(#p{})}}", level - 1));
            }
        }
        styled.push_str(
            r#"</style></defs><rect width="10" height="10" style="fill: url('#p8')"/></svg>"#,
        );
        assert!(
            format!("{:#}", prescan_svg(&styled).unwrap_err()).contains("<style>"),
            "references in a <style> are refused"
        );
        let plain_style = r#"<svg xmlns="http://www.w3.org/2000/svg"><style>rect{fill:red}</style><rect width="1" height="1"/></svg>"#;
        prescan_svg(plain_style).expect("a <style> without url() is fine");

        let cycle =
            r##"<svg xmlns="http://www.w3.org/2000/svg"><g id="a"><use href="#a"/></g></svg>"##;
        assert!(format!("{:#}", prescan_svg(cycle).unwrap_err()).contains("cycle"));

        let deep = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg">{}{}</svg>"#,
            "<g>".repeat(MAX_SVG_NESTING + 1),
            "</g>".repeat(MAX_SVG_NESTING + 1)
        );
        assert!(prescan_svg(&deep).is_err(), "deep nesting");

        let ordinary = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
            <defs><linearGradient id="g"><stop offset="0" stop-color="red"/></linearGradient>
            <circle id="dot" r="1"/></defs>
            <!-- a comment with <use href="#x"/> in it -->
            <use href="#dot" x="2"/><use href="#dot" x="4"/>
            <rect width="10" height="10" fill="url(#g)"/></svg>"##;
        decode_svg(ordinary, 10, 10, None, Default::default()).expect("an ordinary drawing");
    }

    /// integration-5 S3: assets get the byte checks too, so an `.svgz` asset is refused.
    #[test]
    fn test_svgz_asset_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let assets = dir.path().join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(assets.join("bomb.svg"), [0x1f, 0x8b, 8, 0, 0, 0, 0, 0]).unwrap();
        let job = DecodeJob {
            input: DecodeInput::Asset {
                dir: dir.path().to_path_buf(),
                path: "assets/bomb.svg".into(),
            },
            pixels: Some((10, 10)),
            tint: None,
            fonts: Default::default(),
        };
        let error = decode_job(job).expect_err("svgz");
        assert!(format!("{error:#}").contains("svgz"), "{error:#}");
    }

    fn settle(cx: &mut TestAppContext) {
        for _ in 0..50 {
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(5));
        }
        cx.run_until_parked();
    }

    /// The cache: decoded off the main thread, kept per extension within 64 MiB (the least
    /// recently drawn first out), and gone with `remove-image`.
    #[gpui::test]
    async fn test_cache_decodes_evicts_and_forgets(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let cache = cx.new(|_| ImageCache::default());
        let big = |n: u32| -> Arc<[u8]> {
            format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="{n}" fill="#123456"/></svg>"##
            )
            .into_bytes()
            .into()
        };
        // Five full-size images (16 MiB each) for a 64 MiB budget.
        let mut keys = Vec::new();
        for handle in 0..5u32 {
            let key = cache.update(cx, |cache, _| {
                cache.add_generated("ext", handle, ImageFormat::Svg, big(handle + 1));
                cache
                    .key_for(
                        "ext",
                        &ImageSource::Generated(handle),
                        None,
                        (MAX_IMAGE_SIDE, MAX_IMAGE_SIDE),
                        None,
                    )
                    .expect("live handle")
            });
            assert!(matches!(
                cache.read_with(cx, |cache, _| cache.get("ext", &key)),
                ImageState::Pending
            ));
            cache.update(cx, |cache, cx| cache.request_wanted(cx));
            settle(cx);
            assert!(
                matches!(
                    cache.read_with(cx, |cache, _| cache.get("ext", &key)),
                    ImageState::Ready(_)
                ),
                "image {handle} decoded"
            );
            keys.push(key);
        }
        assert_eq!(
            cache.read_with(cx, |cache, _| cache.used_bytes("ext")),
            MAX_DECODED_BYTES_PER_EXTENSION
        );
        // The first, drawn longest ago, made room for the fifth.
        cache.read_with(cx, |cache, _| {
            let images = &cache.extensions["ext"];
            assert!(!images.decoded.contains_key(&keys[0]));
            assert!(images.decoded.contains_key(&keys[4]));
        });

        // remove-image: gone, and its key no longer resolves.
        cache.update(cx, |cache, cx| cache.remove_generated("ext", 4, cx));
        cache.read_with(cx, |cache, _| {
            assert!(!cache.extensions["ext"].decoded.contains_key(&keys[4]));
            assert!(
                cache
                    .key_for("ext", &ImageSource::Generated(4), None, (8, 8), None)
                    .is_none()
            );
        });
        // A reload clears the rest.
        cache.update(cx, |cache, cx| cache.clear("ext", cx));
        assert_eq!(cache.read_with(cx, |cache, _| cache.used_bytes("ext")), 0);
    }

    /// Assets come from the extension's folder only, at most 2 MiB.
    #[test]
    fn test_assets_stay_in_their_folder() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("ext");
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("assets/logo.png"), png(2, 2)).unwrap();
        std::fs::write(root.path().join("secret.png"), png(2, 2)).unwrap();
        assert!(read_asset(&dir, "assets/logo.png").is_ok());
        assert!(read_asset(&dir, "../secret.png").is_err());
        std::fs::write(dir.join("assets/big.png"), vec![0u8; MAX_ASSET_BYTES + 1]).unwrap();
        assert!(read_asset(&dir, "assets/big.png").is_err());
    }
}
