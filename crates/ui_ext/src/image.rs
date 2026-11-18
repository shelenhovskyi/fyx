//! Images of `fyx:ui@0.3.0` (docs/fyx/wit-0.3/design.md, section 12): the `image` node's props,
//! the limits the host and the renderer share, and the checks that need no decoder.
//!
//! The host checks props, asset paths and the bytes of `images.add-image` here, before anything
//! is allocated for pixels: a PNG's IHDR is read and anything over [`MAX_IMAGE_SIDE`] is refused,
//! and an SVG that declares entities (the "billion laughs" bomb) or is compressed is refused. The
//! renderer's decoder (plan 4.11) applies the rest of the limits while it decodes.

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::types::ColorToken;
use crate::v0_2::{HostErrorCode, SurfaceKind};

/// The manifest capability for `image` nodes and the `images` interface (tier T0).
pub const CAPABILITY_IMAGES: &str = "ui:images";
/// Package assets live in this folder of the extension's version folder.
pub const ASSETS_DIR: &str = "assets";
/// Bytes of an asset file on disk.
pub const MAX_ASSET_BYTES: usize = 2 << 20;
/// Bytes of one `add-image` call.
pub const MAX_GENERATED_IMAGE_BYTES: usize = 1 << 20;
/// Live generated images per extension.
pub const MAX_GENERATED_IMAGES: usize = 32;
/// Bytes in an asset path.
pub const MAX_ASSET_PATH_BYTES: usize = 1024;
/// Bytes in `alt`, which is required.
pub const MAX_ALT_BYTES: usize = 256;
/// The longest side, in logical pixels for the box and in pixels for the decoded image (after
/// the device scale factor).
pub const MAX_IMAGE_SIDE: u32 = 2048;
/// RGBA bytes of one decoded image at [`MAX_IMAGE_SIDE`] square: 16 MiB.
pub const MAX_DECODED_IMAGE_BYTES: u64 = MAX_IMAGE_SIDE as u64 * MAX_IMAGE_SIDE as u64 * 4;
/// Decoded pixels one extension may hold, for the renderer's LRU (decided, design.md Q13).
pub const MAX_DECODED_BYTES_PER_EXTENSION: u64 = 64 << 20;
/// A decode that takes longer leaves the alt text and an error icon.
pub const DECODE_TIMEOUT: Duration = Duration::from_millis(250);
pub const MAX_CONCURRENT_DECODES_PER_EXTENSION: usize = 2;
pub const MAX_CONCURRENT_DECODES: usize = 4;
/// Nodes of a parsed SVG tree (renderer).
pub const MAX_SVG_NODES: usize = 100_000;
/// Nesting levels of `<use>` (renderer).
pub const MAX_SVG_USE_DEPTH: usize = 16;
/// Standard deviation of a blur filter (renderer).
pub const MAX_SVG_BLUR_STD_DEVIATION: f32 = 64.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImageFormat {
    Png,
    Svg,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImageSource {
    /// A file in the package's `assets/` folder; see [`check_asset_path`].
    Asset(String),
    /// A handle from `images.add-image`.
    Generated(u32),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImageFit {
    #[default]
    Contain,
    Cover,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ImageProps {
    pub source: ImageSource,
    pub alt: String,
    /// Logical pixels, 1 to [`MAX_IMAGE_SIDE`].
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fit: ImageFit,
    /// SVG only: drawn as a one-colour mask in this colour.
    pub tint: Option<ColorToken>,
}

/// What the cheap checks learned about an image's bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageInfo {
    pub format: ImageFormat,
    /// From the PNG header; `None` for SVG, whose size is only known after parsing.
    pub size: Option<(u32, u32)>,
}

impl ImageInfo {
    /// RGBA bytes once decoded at its own size, when the header says it.
    pub fn decoded_bytes(&self) -> Option<u64> {
        self.size
            .map(|(width, height)| u64::from(width) * u64::from(height) * 4)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ImageError {
    #[error("an image needs alt text of 1 to {MAX_ALT_BYTES} bytes, got {0} bytes")]
    Alt(usize),
    #[error("an image's {side} is {value}, it must be 1 to {MAX_IMAGE_SIDE}")]
    BoxSize { side: &'static str, value: u32 },
    #[error("image asset path {path:?} {problem}")]
    AssetPath { path: String, problem: &'static str },
    #[error("{format:?} image has {bytes} bytes, the cap is {max}")]
    TooManyBytes {
        format: ImageFormat,
        bytes: usize,
        max: usize,
    },
    #[error("image is {width} x {height} pixels, the cap is {MAX_IMAGE_SIDE} x {MAX_IMAGE_SIDE}")]
    TooManyPixels { width: u32, height: u32 },
    #[error("not a {format:?} image: {reason}")]
    Malformed {
        format: ImageFormat,
        reason: &'static str,
    },
    #[error("an image can only be in a panel, not in a {0}")]
    NotInPanel(&'static str),
    #[error("an extension has at most {MAX_GENERATED_IMAGES} generated images")]
    TooManyImages,
}

impl ImageError {
    /// The WIT `host-error-code` for this error.
    pub fn code(&self) -> HostErrorCode {
        match self {
            ImageError::TooManyBytes { .. }
            | ImageError::TooManyPixels { .. }
            | ImageError::TooManyImages => HostErrorCode::LimitExceeded,
            ImageError::Alt(_)
            | ImageError::BoxSize { .. }
            | ImageError::AssetPath { .. }
            | ImageError::Malformed { .. }
            | ImageError::NotInPanel(_) => HostErrorCode::InvalidArgument,
        }
    }
}

/// The props rules that need no file: alt text, box sizes, and the asset path syntax.
pub fn check_image_props(props: &ImageProps) -> Result<(), ImageError> {
    if !(1..=MAX_ALT_BYTES).contains(&props.alt.len()) {
        return Err(ImageError::Alt(props.alt.len()));
    }
    for (side, value) in [("width", props.width), ("height", props.height)] {
        if let Some(value) = value
            && !(1..=MAX_IMAGE_SIDE).contains(&value)
        {
            return Err(ImageError::BoxSize { side, value });
        }
    }
    if let ImageSource::Asset(path) = &props.source {
        check_asset_path(path)?;
    }
    Ok(())
}

/// An asset path: `assets/…`, relative, `/`-separated, with no empty, `.` or `..` component, no
/// `\`, `:` or NUL (a Windows separator, a drive, never a file name), at most
/// [`MAX_ASSET_PATH_BYTES`], ending in `.png` or `.svg`. Returns the format its extension names.
/// Whether it resolves inside the version folder is the host's check on disk.
pub fn check_asset_path(path: &str) -> Result<ImageFormat, ImageError> {
    let problem = |problem| ImageError::AssetPath {
        path: path.to_owned(),
        problem,
    };
    if path.len() > MAX_ASSET_PATH_BYTES {
        return Err(problem("is too long"));
    }
    if path.contains(['\\', ':', '\0']) {
        return Err(problem("contains '\\', ':' or NUL"));
    }
    let mut components = path.split('/');
    if components.next() != Some(ASSETS_DIR) {
        return Err(problem("must start with assets/"));
    }
    let mut last = None;
    for component in components {
        if component.is_empty() || component == "." || component == ".." {
            return Err(problem("has an empty, '.' or '..' component"));
        }
        last = Some(component);
    }
    let file = last.ok_or_else(|| problem("names no file"))?;
    let extension = file.rsplit_once('.').map(|(_, extension)| extension);
    match extension.map(str::to_ascii_lowercase).as_deref() {
        Some("png") => Ok(ImageFormat::Png),
        Some("svg") => Ok(ImageFormat::Svg),
        _ => Err(problem("must end in .png or .svg")),
    }
}

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

/// The checks of an image's bytes that allocate nothing: the size cap, then for PNG the signature
/// and the IHDR (dimensions at most [`MAX_IMAGE_SIDE`], a valid bit depth and colour type), for
/// SVG UTF-8 text with an `<svg` element, no entity declarations and no gzip.
pub fn check_image_bytes(
    format: ImageFormat,
    bytes: &[u8],
    max_bytes: usize,
) -> Result<ImageInfo, ImageError> {
    if bytes.len() > max_bytes {
        return Err(ImageError::TooManyBytes {
            format,
            bytes: bytes.len(),
            max: max_bytes,
        });
    }
    match format {
        ImageFormat::Png => check_png(bytes),
        ImageFormat::Svg => check_svg(bytes),
    }
}

fn check_png(bytes: &[u8]) -> Result<ImageInfo, ImageError> {
    let malformed = |reason| ImageError::Malformed {
        format: ImageFormat::Png,
        reason,
    };
    // Signature, then the IHDR chunk: length 13, type, width, height, bit depth, colour type.
    if bytes.len() < 8 + 8 + 13 || bytes[..8] != PNG_SIGNATURE {
        return Err(malformed("no PNG signature"));
    }
    let be32 =
        |at: usize| u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    if be32(8) != 13 || &bytes[12..16] != b"IHDR" {
        return Err(malformed("the first chunk is not IHDR"));
    }
    let (width, height) = (be32(16), be32(20));
    if width == 0 || height == 0 {
        return Err(malformed("zero width or height"));
    }
    if width > MAX_IMAGE_SIDE || height > MAX_IMAGE_SIDE {
        return Err(ImageError::TooManyPixels { width, height });
    }
    let (bit_depth, color_type) = (bytes[24], bytes[25]);
    let valid = match color_type {
        0 => matches!(bit_depth, 1 | 2 | 4 | 8 | 16),
        3 => matches!(bit_depth, 1 | 2 | 4 | 8),
        2 | 4 | 6 => matches!(bit_depth, 8 | 16),
        _ => false,
    };
    if !valid {
        return Err(malformed("invalid bit depth or colour type"));
    }
    Ok(ImageInfo {
        format: ImageFormat::Png,
        size: Some((width, height)),
    })
}

fn check_svg(bytes: &[u8]) -> Result<ImageInfo, ImageError> {
    let malformed = |reason| ImageError::Malformed {
        format: ImageFormat::Svg,
        reason,
    };
    if bytes.starts_with(&[0x1f, 0x8b]) {
        return Err(malformed("compressed SVG (svgz) is not supported"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| malformed("not UTF-8 text"))?;
    if !text.contains("<svg") {
        return Err(malformed("no <svg> element"));
    }
    // Entity declarations are how "billion laughs" expands; SVG never needs them.
    if text.contains("<!ENTITY") {
        return Err(malformed("declares XML entities"));
    }
    Ok(ImageInfo {
        format: ImageFormat::Svg,
        size: None,
    })
}

/// Images go only in panel trees and panel list rows: status items, modals, pickers and
/// decorations sit next to host UI and would be easy to spoof (design.md §12, threats).
pub fn check_image_placement(kind: Option<&SurfaceKind>) -> Result<(), ImageError> {
    match kind {
        Some(SurfaceKind::Panel(_)) => Ok(()),
        Some(SurfaceKind::StatusItem(_)) => Err(ImageError::NotInPanel("status item")),
        Some(SurfaceKind::Modal(_)) => Err(ImageError::NotInPanel("modal")),
        None => Err(ImageError::NotInPanel("decoration")),
    }
}

/// Elements nested deeper than this are refused by the scan (the expansion recurses per level).
pub const MAX_SVG_NESTING: usize = 256;

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
pub fn prescan_svg(text: &str) -> Result<(), String> {
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
    let check_text = |text: &str, open: &[(usize, bool)]| -> Result<(), String> {
        if open.last().is_some_and(|(_, style)| *style) && decode_references(text).contains("url(")
        {
            return Err(
                "a <style> in an SVG may not use url(…); use the element's fill or style"
                    .to_string(),
            );
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
        let end = tag_end(rest).ok_or_else(|| "an SVG tag is not closed".to_string())?;
        let tag = &rest[1..end];
        let self_closing = tag.ends_with('/');
        let name = tag
            .split(|character: char| character.is_whitespace() || character == '/')
            .next()
            .unwrap_or_default();
        let is_style = name.rsplit(':').next() == Some("style");
        let index = elements.len();
        if index >= MAX_SVG_NODES {
            return Err(format!("the SVG has more than {MAX_SVG_NODES} nodes"));
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
                return Err(format!(
                    "the SVG nests more than {MAX_SVG_NESTING} elements deep"
                ));
            }
        }
        at = start + end + 1;
    }

    /// (nodes it expands to, longest chain of references below it). `depth` is the references
    /// followed to get here: past [`MAX_SVG_USE_DEPTH`] it is refused before recursing, so the
    /// recursion is at most [`MAX_SVG_NESTING`] times that deep, however long a flat chain of
    /// references the SVG has (which would otherwise recurse once per element and overflow the
    /// stack).
    fn expand(
        index: usize,
        depth: usize,
        elements: &[Element],
        ids: &HashMap<String, usize>,
        memo: &mut [Option<(usize, usize)>],
        visiting: &mut [bool],
    ) -> Result<(usize, usize), String> {
        if let Some(known) = memo[index] {
            return Ok(known);
        }
        if visiting[index] {
            return Err("the SVG has a reference cycle".to_string());
        }
        visiting[index] = true;
        let mut nodes = 1usize;
        let mut chain = 0;
        for &child in &elements[index].children {
            let (child_nodes, child_chain) = expand(child, depth, elements, ids, memo, visiting)?;
            nodes = nodes.saturating_add(child_nodes);
            chain = chain.max(child_chain);
            if nodes > MAX_SVG_NODES {
                return Err(format!(
                    "the SVG has more than {MAX_SVG_NODES} nodes once its references expand"
                ));
            }
        }
        for reference in &elements[index].references {
            let Some(&target) = ids.get(reference) else {
                continue;
            };
            if depth + 1 > MAX_SVG_USE_DEPTH {
                return Err(format!(
                    "the SVG nests references more than {MAX_SVG_USE_DEPTH} deep"
                ));
            }
            let (target_nodes, target_chain) =
                expand(target, depth + 1, elements, ids, memo, visiting)?;
            nodes = nodes.saturating_add(target_nodes);
            chain = chain.max(target_chain + 1);
            if chain > MAX_SVG_USE_DEPTH {
                return Err(format!(
                    "the SVG nests references more than {MAX_SVG_USE_DEPTH} deep"
                ));
            }
            if nodes > MAX_SVG_NODES {
                return Err(format!(
                    "the SVG has more than {MAX_SVG_NODES} nodes once its references expand"
                ));
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
        let (nodes, _) = expand(root, 0, &elements, &ids, &mut memo, &mut visiting)?;
        total = total.saturating_add(nodes);
        if total > MAX_SVG_NODES {
            return Err(format!(
                "the SVG has more than {MAX_SVG_NODES} nodes once its references expand"
            ));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// integration-5 S3: the scan before `usvg` refuses what would expand past the caps, and a
    /// long flat chain of references fails on depth without recursing once per element.
    #[test]
    fn prescan_svg_bounds_expansion_and_recursion() {
        let svg = |body: &str| format!("<svg xmlns=\"http://www.w3.org/2000/svg\">{body}</svg>");
        prescan_svg(&svg("<rect id=\"a\"/><use href=\"#a\"/>")).expect("small");

        // Ten levels of ten <use>s: 10^10 nodes once expanded.
        let mut body = String::from("<g id=\"l0\"><rect/></g>");
        for level in 1..10 {
            body.push_str(&format!("<g id=\"l{level}\">"));
            for _ in 0..10 {
                body.push_str(&format!("<use href=\"#l{}\"/>", level - 1));
            }
            body.push_str("</g>");
        }
        let error = prescan_svg(&svg(&body)).unwrap_err();
        assert!(error.contains("nodes"), "{error}");

        let error = prescan_svg(&svg("<g id=\"a\"><use href=\"#b\"/></g><g id=\"b\"><use \
             xlink:href=\"&#35;a\"/></g>"))
        .unwrap_err();
        assert!(error.contains("cycle"), "{error}");

        let error = prescan_svg(&svg("<style>rect { fill: url(#p) }</style><rect/>")).unwrap_err();
        assert!(error.contains("<style>"), "{error}");

        // 40000 elements, each referring to the next: refused on depth, on this thread's stack.
        let mut chain = String::new();
        for index in 0..40_000 {
            chain.push_str(&format!(
                "<g id=\"c{index}\" fill=\"url(#c{})\"/>",
                index + 1
            ));
        }
        let error = prescan_svg(&svg(&chain)).unwrap_err();
        assert!(error.contains("references more than"), "{error}");

        let error = prescan_svg(&svg(&"<g>".repeat(MAX_SVG_NESTING + 1))).unwrap_err();
        assert!(error.contains("nests more than"), "{error}");
    }
    use crate::v0_2::{
        DockPosition, Icon, IconRef, ModalDescriptor, ModalWidth, PanelDescriptor, StatusBarSide,
        StatusItemDescriptor,
    };

    fn png_header(width: u32, height: u32, bit_depth: u8, color_type: u8) -> Vec<u8> {
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.extend(13u32.to_be_bytes());
        bytes.extend(b"IHDR");
        bytes.extend(width.to_be_bytes());
        bytes.extend(height.to_be_bytes());
        bytes.extend([bit_depth, color_type, 0, 0, 0]);
        bytes.extend([0; 4]); // CRC, not checked here
        bytes
    }

    fn props(source: ImageSource) -> ImageProps {
        ImageProps {
            source,
            alt: "Logo".into(),
            width: Some(64),
            height: None,
            fit: ImageFit::Contain,
            tint: None,
        }
    }

    #[test]
    fn test_png_header_checks() {
        let info = check_image_bytes(ImageFormat::Png, &png_header(64, 32, 8, 6), MAX_ASSET_BYTES)
            .expect("valid");
        assert_eq!(info.size, Some((64, 32)));
        assert_eq!(info.decoded_bytes(), Some(64 * 32 * 4));
        check_image_bytes(
            ImageFormat::Png,
            &png_header(2048, 2048, 16, 2),
            MAX_ASSET_BYTES,
        )
        .expect("the cap itself is allowed");

        // The 1 KiB bomb: its header claims 65535 × 65535; refused before any pixel is allocated.
        let mut bomb = png_header(65535, 65535, 8, 6);
        bomb.resize(1024, 0);
        assert_eq!(
            check_image_bytes(ImageFormat::Png, &bomb, MAX_ASSET_BYTES),
            Err(ImageError::TooManyPixels {
                width: 65535,
                height: 65535
            })
        );
        assert_eq!(
            check_image_bytes(ImageFormat::Png, &bomb, MAX_ASSET_BYTES)
                .unwrap_err()
                .code(),
            HostErrorCode::LimitExceeded
        );
        for bad in [
            png_header(2049, 1, 8, 6),
            png_header(0, 10, 8, 6),
            png_header(10, 10, 3, 6),
            png_header(10, 10, 16, 3),
            png_header(10, 10, 8, 5),
            b"GIF89a".repeat(10),
            png_header(10, 10, 8, 6)[..20].to_vec(),
        ] {
            assert!(
                check_image_bytes(ImageFormat::Png, &bad, MAX_ASSET_BYTES).is_err(),
                "{bad:?}"
            );
        }
        let big = {
            let mut bytes = png_header(10, 10, 8, 6);
            bytes.resize(MAX_GENERATED_IMAGE_BYTES + 1, 0);
            bytes
        };
        assert!(matches!(
            check_image_bytes(ImageFormat::Png, &big, MAX_GENERATED_IMAGE_BYTES),
            Err(ImageError::TooManyBytes { .. })
        ));
    }

    #[test]
    fn test_svg_checks() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"/>"#;
        assert_eq!(
            check_image_bytes(ImageFormat::Svg, svg, MAX_GENERATED_IMAGE_BYTES)
                .expect("valid")
                .size,
            None
        );
        let laughs = br#"<?xml version="1.0"?><!DOCTYPE lolz [<!ENTITY lol "lol"><!ENTITY lol2 "&lol;&lol;&lol;&lol;">]><svg>&lol2;</svg>"#;
        for bad in [
            &laughs[..],
            &[0x1f, 0x8b, 8, 0][..],
            &[0xff, 0xfe, b'<'][..],
            b"<html></html>",
        ] {
            let error = check_image_bytes(ImageFormat::Svg, bad, MAX_GENERATED_IMAGE_BYTES)
                .expect_err("refused");
            assert_eq!(error.code(), HostErrorCode::InvalidArgument, "{error}");
        }
    }

    #[test]
    fn test_asset_paths() {
        assert_eq!(check_asset_path("assets/logo.svg"), Ok(ImageFormat::Svg));
        assert_eq!(
            check_asset_path("assets/icons/Logo.PNG"),
            Ok(ImageFormat::Png)
        );
        for bad in [
            "logo.png",
            "/assets/logo.png",
            "assets/../extension.toml",
            "assets/./logo.png",
            "assets//logo.png",
            "assets/..\\..\\secret.png",
            "assets\\logo.png",
            "C:/assets/logo.png",
            "assets/C:logo.png",
            "assets/logo.jpg",
            "assets/logo",
            "assets/",
            "assets",
            "file:///etc/passwd.png",
            "assets/logo.png\0",
        ] {
            assert!(check_asset_path(bad).is_err(), "{bad:?}");
        }
        assert!(check_asset_path(&format!("assets/{}.png", "a".repeat(1024))).is_err());
    }

    #[test]
    fn test_props_and_placement() {
        check_image_props(&props(ImageSource::Asset("assets/logo.svg".into()))).expect("valid");
        check_image_props(&props(ImageSource::Generated(3))).expect("valid");
        let no_alt = ImageProps {
            alt: String::new(),
            ..props(ImageSource::Generated(1))
        };
        assert_eq!(check_image_props(&no_alt), Err(ImageError::Alt(0)));
        let long_alt = ImageProps {
            alt: "a".repeat(257),
            ..props(ImageSource::Generated(1))
        };
        assert!(check_image_props(&long_alt).is_err());
        for (width, height) in [(Some(0), None), (None, Some(2049))] {
            let sized = ImageProps {
                width,
                height,
                ..props(ImageSource::Generated(1))
            };
            assert!(matches!(
                check_image_props(&sized),
                Err(ImageError::BoxSize { .. })
            ));
        }
        assert!(check_image_props(&props(ImageSource::Asset("../x.png".into()))).is_err());

        let panel = SurfaceKind::Panel(PanelDescriptor {
            position: DockPosition::Left,
            icon: IconRef::Frozen(Icon::Image),
            default_size: 300,
        });
        check_image_placement(Some(&panel)).expect("panels may show images");
        let status = SurfaceKind::StatusItem(StatusItemDescriptor {
            side: StatusBarSide::Left,
            priority: 0,
        });
        let modal = SurfaceKind::Modal(ModalDescriptor {
            width: ModalWidth::Small,
        });
        for kind in [Some(&status), Some(&modal), None] {
            assert_eq!(
                check_image_placement(kind).unwrap_err().code(),
                HostErrorCode::InvalidArgument
            );
        }
    }

    #[test]
    fn test_json_form() {
        let props = props(ImageSource::Asset("assets/logo.svg".into()));
        let json = serde_json::to_value(&props).expect("serializes");
        assert_eq!(
            json,
            serde_json::json!({
                "source": {"asset": "assets/logo.svg"},
                "alt": "Logo",
                "width": 64,
                "height": null,
                "fit": "contain",
                "tint": null,
            })
        );
        assert_eq!(serde_json::from_value::<ImageProps>(json).ok(), Some(props));
    }
}
