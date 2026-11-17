//! Generates the extension API reference pages in `docs/fyx/api/` from the sources they describe,
//! and checks the committed pages and the hand-written `docs/fyx/sdk-reference.md` for drift
//! (platform spec decision 11, plan task 1.9).
//!
//! - The WIT page comes from the `fyx:ui` package via `wit-parser`, doc comments included.
//! - The Rust pages list every public item of `ui_ext_sdk` and `fyx_ext_harness`, read with
//!   `syn` from their source, with the first line of each doc comment. Full signatures are in the
//!   rustdoc output that `script/fyx-api-docs` also builds.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use wit_parser::{Docs, Function, Resolve, Type, TypeDefKind, TypeOwner, WorldItem, WorldKey};

/// One generated page: where it goes, relative to the repository root, and its content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub path: PathBuf,
    pub content: String,
}

pub const API_DIR: &str = "docs/fyx/api";
pub const HAND_WRITTEN_REFERENCE: &str = "docs/fyx/sdk-reference.md";

/// Kebab-case names in prose that are tools or crates, not WIT items, so the drift check accepts
/// them.
pub const NOT_WIT_NAMES: &[&str] = &["wit-bindgen", "wasm-tools", "fyx-ext", "fyx-api-docs"];

/// The Rust crates whose public items get a page: (crate name, root source file).
pub const RUST_CRATES: &[(&str, &str)] = &[
    ("ui_ext_sdk", "crates/ui_ext_sdk/src/ui_ext_sdk.rs"),
    (
        "fyx_ext_harness",
        "crates/fyx_ext_harness/src/fyx_ext_harness.rs",
    ),
];

/// The WIT packages that get a page: the directory with the package.
pub const WIT_DIRS: &[&str] = &["crates/ui_ext/wit"];

pub fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every page, generated from the sources under `root`.
pub fn generate(root: &Path) -> Result<Vec<Page>> {
    let mut pages = Vec::new();
    for dir in WIT_DIRS {
        let (name, content) = wit_page(&root.join(dir), dir)?;
        pages.push(Page {
            path: Path::new(API_DIR).join(format!("{name}.md")),
            content,
        });
    }
    for (crate_name, source) in RUST_CRATES {
        let items = rust_items(&root.join(source))?;
        pages.push(Page {
            path: Path::new(API_DIR).join(format!("{crate_name}.md")),
            content: rust_page(crate_name, source, &items),
        });
    }
    Ok(pages)
}

fn header(title: &str, source: &str) -> String {
    format!(
        "# {title}\n\nGenerated from `{source}` by `script/fyx-api-docs`; do not edit by hand. A test \
         fails when this page and its source disagree.\n\n"
    )
}

fn doc_text(docs: &Docs) -> String {
    docs.contents
        .as_deref()
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn table_cell(text: &str) -> String {
    text.replace('|', "\\|")
}

fn type_name(resolve: &Resolve, ty: &Type) -> String {
    match ty {
        Type::Bool => "bool".into(),
        Type::U8 => "u8".into(),
        Type::U16 => "u16".into(),
        Type::U32 => "u32".into(),
        Type::U64 => "u64".into(),
        Type::S8 => "s8".into(),
        Type::S16 => "s16".into(),
        Type::S32 => "s32".into(),
        Type::S64 => "s64".into(),
        Type::F32 => "f32".into(),
        Type::F64 => "f64".into(),
        Type::Char => "char".into(),
        Type::String => "string".into(),
        Type::ErrorContext => "error-context".into(),
        Type::Id(id) => {
            let def = &resolve.types[*id];
            if let Some(name) = &def.name {
                return name.clone();
            }
            match &def.kind {
                TypeDefKind::List(inner) => format!("list<{}>", type_name(resolve, inner)),
                TypeDefKind::Option(inner) => format!("option<{}>", type_name(resolve, inner)),
                TypeDefKind::Tuple(tuple) => format!(
                    "tuple<{}>",
                    tuple
                        .types
                        .iter()
                        .map(|ty| type_name(resolve, ty))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                TypeDefKind::Result(result) => {
                    let ok = result
                        .ok
                        .as_ref()
                        .map_or("_".into(), |ty| type_name(resolve, ty));
                    let err = result
                        .err
                        .as_ref()
                        .map_or("_".into(), |ty| type_name(resolve, ty));
                    format!("result<{ok}, {err}>")
                }
                TypeDefKind::Type(inner) => type_name(resolve, inner),
                other => format!("{other:?}"),
            }
        }
    }
}

fn signature(resolve: &Resolve, function: &Function) -> String {
    let params = function
        .params
        .iter()
        .map(|param| format!("{}: {}", param.name, type_name(resolve, &param.ty)))
        .collect::<Vec<_>>()
        .join(", ");
    match &function.result {
        Some(result) => format!(
            "{}: func({params}) -> {}",
            function.name,
            type_name(resolve, result)
        ),
        None => format!("{}: func({params})", function.name),
    }
}

/// The reference page of the WIT package in `dir`; returns `fyx-ui-<version>` and the page.
pub fn wit_page(dir: &Path, source: &str) -> Result<(String, String)> {
    let mut resolve = Resolve::default();
    let (package_id, _) = resolve
        .push_dir(dir)
        .with_context(|| format!("parsing the WIT in {dir:?}"))?;
    let package = &resolve.packages[package_id];
    let version = package
        .name
        .version
        .as_ref()
        .map_or("unversioned".to_string(), ToString::to_string);
    let full_name = package.name.to_string();
    let mut page = header(&format!("`{full_name}` WIT reference"), source);

    for (world_name, world_id) in &package.worlds {
        let world = &resolve.worlds[*world_id];
        writeln!(page, "## World `{world_name}`\n").ok();
        let docs = doc_text(&world.docs);
        if !docs.is_empty() {
            writeln!(page, "{docs}\n").ok();
        }
        for (label, items) in [("Imports", &world.imports), ("Exports", &world.exports)] {
            writeln!(page, "{label}:\n").ok();
            for (key, item) in items {
                let line = match (key, item) {
                    (_, WorldItem::Interface { id, .. }) => {
                        format!(
                            "- interface `{}`",
                            resolve
                                .id_of(*id)
                                .unwrap_or_else(|| resolve.name_world_key(key))
                        )
                    }
                    (_, WorldItem::Function(function)) => {
                        let docs = doc_text(&function.docs);
                        let mut line = format!("- `{}`", signature(&resolve, function));
                        if !docs.is_empty() {
                            write!(line, ": {docs}").ok();
                        }
                        line
                    }
                    (WorldKey::Name(name), WorldItem::Type { .. }) => format!("- type `{name}`"),
                    (WorldKey::Interface(_), WorldItem::Type { id, .. }) => {
                        format!(
                            "- type `{}`",
                            resolve.types[*id].name.clone().unwrap_or_default()
                        )
                    }
                };
                writeln!(page, "{line}").ok();
            }
            writeln!(page).ok();
        }
    }

    for (interface_name, interface_id) in &package.interfaces {
        let interface = &resolve.interfaces[*interface_id];
        writeln!(page, "## Interface `{interface_name}`\n").ok();
        let docs = doc_text(&interface.docs);
        if !docs.is_empty() {
            writeln!(page, "{docs}\n").ok();
        }
        for (type_name_text, type_id) in &interface.types {
            let def = &resolve.types[*type_id];
            if !matches!(def.owner, TypeOwner::Interface(owner) if owner == *interface_id) {
                continue;
            }
            let docs = doc_text(&def.docs);
            match &def.kind {
                TypeDefKind::Record(record) => {
                    writeln!(page, "### record `{type_name_text}`\n").ok();
                    if !docs.is_empty() {
                        writeln!(page, "{docs}\n").ok();
                    }
                    writeln!(page, "| Field | Type | Notes |\n|---|---|---|").ok();
                    for field in &record.fields {
                        writeln!(
                            page,
                            "| `{}` | `{}` | {} |",
                            field.name,
                            type_name(&resolve, &field.ty),
                            table_cell(&doc_text(&field.docs))
                        )
                        .ok();
                    }
                }
                TypeDefKind::Variant(variant) => {
                    writeln!(page, "### variant `{type_name_text}`\n").ok();
                    if !docs.is_empty() {
                        writeln!(page, "{docs}\n").ok();
                    }
                    writeln!(page, "| Case | Payload | Notes |\n|---|---|---|").ok();
                    for case in &variant.cases {
                        let payload = case
                            .ty
                            .as_ref()
                            .map_or(String::new(), |ty| format!("`{}`", type_name(&resolve, ty)));
                        writeln!(
                            page,
                            "| `{}` | {payload} | {} |",
                            case.name,
                            table_cell(&doc_text(&case.docs))
                        )
                        .ok();
                    }
                }
                TypeDefKind::Enum(enumeration) => {
                    writeln!(page, "### enum `{type_name_text}`\n").ok();
                    if !docs.is_empty() {
                        writeln!(page, "{docs}\n").ok();
                    }
                    let cases = enumeration
                        .cases
                        .iter()
                        .map(|case| format!("`{}`", case.name))
                        .collect::<Vec<_>>()
                        .join(", ");
                    writeln!(page, "Cases: {cases}.").ok();
                }
                kind => {
                    let target = match kind {
                        TypeDefKind::Type(ty) => type_name(&resolve, ty),
                        other => format!("{other:?}"),
                    };
                    writeln!(page, "### type `{type_name_text}` = `{target}`\n").ok();
                    if !docs.is_empty() {
                        writeln!(page, "{docs}").ok();
                    }
                }
            }
            writeln!(page).ok();
        }
        for function in interface.functions.values() {
            writeln!(page, "### `{}`\n", signature(&resolve, function)).ok();
            let docs = doc_text(&function.docs);
            if !docs.is_empty() {
                writeln!(page, "{docs}\n").ok();
            }
        }
    }
    Ok((
        format!("fyx-ui-{version}"),
        page.trim_end().to_string() + "\n",
    ))
}

/// Every name the WIT package in `dir` defines: interfaces, worlds, types, fields, cases and
/// functions.
pub fn wit_names(dir: &Path) -> Result<BTreeSet<String>> {
    let mut resolve = Resolve::default();
    let (package_id, _) = resolve
        .push_dir(dir)
        .with_context(|| format!("parsing the WIT in {dir:?}"))?;
    let package = &resolve.packages[package_id];
    let mut names = BTreeSet::new();
    names.extend(package.worlds.keys().cloned());
    for world_id in package.worlds.values() {
        let world = &resolve.worlds[*world_id];
        for item in world.imports.values().chain(world.exports.values()) {
            if let WorldItem::Function(function) = item {
                names.insert(function.name.clone());
            }
        }
    }
    for (interface_name, interface_id) in &package.interfaces {
        names.insert(interface_name.clone());
        let interface = &resolve.interfaces[*interface_id];
        for (type_name_text, type_id) in &interface.types {
            names.insert(type_name_text.clone());
            match &resolve.types[*type_id].kind {
                TypeDefKind::Record(record) => {
                    names.extend(record.fields.iter().map(|f| f.name.clone()))
                }
                TypeDefKind::Variant(variant) => {
                    names.extend(variant.cases.iter().map(|c| c.name.clone()))
                }
                TypeDefKind::Enum(enumeration) => {
                    names.extend(enumeration.cases.iter().map(|c| c.name.clone()))
                }
                _ => {}
            }
        }
        names.extend(interface.functions.keys().cloned());
    }
    Ok(names)
}

/// A public item of a Rust crate, as an author can name it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RustItem {
    /// `SurfaceTree::set_props`, `register_extension!`, `bindings` (a module).
    pub path: String,
    pub kind: &'static str,
    /// The first line of its doc comment.
    pub summary: String,
    /// `host only` for items behind `cfg(not(target_arch = "wasm32"))`.
    pub note: Option<&'static str>,
}

fn has_attribute(attributes: &[syn::Attribute], name: &str, contains: &str) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident(name)
            && match &attribute.meta {
                syn::Meta::List(list) => {
                    list.tokens.to_string().replace(' ', "").contains(contains)
                }
                syn::Meta::Path(_) => contains.is_empty(),
                syn::Meta::NameValue(_) => false,
            }
    })
}

fn is_hidden(attributes: &[syn::Attribute]) -> bool {
    has_attribute(attributes, "doc", "hidden") || has_attribute(attributes, "cfg", "test")
}

fn host_only(attributes: &[syn::Attribute]) -> Option<&'static str> {
    has_attribute(attributes, "cfg", "not(target_arch=\"wasm32\")").then_some("host only")
}

/// The first paragraph of the doc comment on one line, with rustdoc link brackets removed.
fn summary(attributes: &[syn::Attribute]) -> String {
    let lines = attributes
        .iter()
        .filter(|attribute| attribute.path().is_ident("doc"))
        .filter_map(|attribute| match &attribute.meta {
            syn::Meta::NameValue(syn::MetaNameValue {
                value:
                    syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(text),
                        ..
                    }),
                ..
            }) => Some(text.value()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let paragraph = lines
        .iter()
        .map(|line| line.trim())
        .take_while(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    paragraph.replace("[`", "`").replace("`]", "`")
}

fn is_public(visibility: &syn::Visibility) -> bool {
    matches!(visibility, syn::Visibility::Public(_))
}

fn type_path(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        _ => None,
    }
}

/// The public items of the crate whose root source file is `path`. Private modules, test
/// modules and `#[doc(hidden)]` items are left out; so are trait impls, which add no names.
pub fn rust_items(path: &Path) -> Result<Vec<RustItem>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {path:?}"))?;
    let file = syn::parse_file(&text).with_context(|| format!("parsing {path:?}"))?;
    let mut items = Vec::new();
    collect_items(&file.items, "", &mut items);
    items.sort();
    items.dedup();
    Ok(items)
}

fn push_item(
    items: &mut Vec<RustItem>,
    prefix: &str,
    name: String,
    kind: &'static str,
    attributes: &[syn::Attribute],
) {
    items.push(RustItem {
        path: format!("{prefix}{name}"),
        kind,
        summary: summary(attributes),
        note: host_only(attributes),
    });
}

fn collect_items(source: &[syn::Item], prefix: &str, items: &mut Vec<RustItem>) {
    for item in source {
        match item {
            syn::Item::Fn(function) if is_public(&function.vis) && !is_hidden(&function.attrs) => {
                push_item(
                    items,
                    prefix,
                    function.sig.ident.to_string(),
                    "fn",
                    &function.attrs,
                )
            }
            syn::Item::Struct(structure)
                if is_public(&structure.vis) && !is_hidden(&structure.attrs) =>
            {
                push_item(
                    items,
                    prefix,
                    structure.ident.to_string(),
                    "struct",
                    &structure.attrs,
                );
                if let syn::Fields::Named(fields) = &structure.fields {
                    for field in &fields.named {
                        if is_public(&field.vis)
                            && let Some(ident) = &field.ident
                        {
                            push_item(
                                items,
                                prefix,
                                format!("{}::{ident}", structure.ident),
                                "field",
                                &field.attrs,
                            );
                        }
                    }
                }
            }
            syn::Item::Enum(enumeration)
                if is_public(&enumeration.vis) && !is_hidden(&enumeration.attrs) =>
            {
                push_item(
                    items,
                    prefix,
                    enumeration.ident.to_string(),
                    "enum",
                    &enumeration.attrs,
                );
                for variant in &enumeration.variants {
                    push_item(
                        items,
                        prefix,
                        format!("{}::{}", enumeration.ident, variant.ident),
                        "variant",
                        &variant.attrs,
                    );
                }
            }
            syn::Item::Const(constant)
                if is_public(&constant.vis) && !is_hidden(&constant.attrs) =>
            {
                push_item(
                    items,
                    prefix,
                    constant.ident.to_string(),
                    "const",
                    &constant.attrs,
                )
            }
            syn::Item::Static(statik) if is_public(&statik.vis) && !is_hidden(&statik.attrs) => {
                push_item(
                    items,
                    prefix,
                    statik.ident.to_string(),
                    "static",
                    &statik.attrs,
                )
            }
            syn::Item::Type(alias) if is_public(&alias.vis) && !is_hidden(&alias.attrs) => {
                push_item(items, prefix, alias.ident.to_string(), "type", &alias.attrs)
            }
            syn::Item::Trait(item_trait)
                if is_public(&item_trait.vis) && !is_hidden(&item_trait.attrs) =>
            {
                push_item(
                    items,
                    prefix,
                    item_trait.ident.to_string(),
                    "trait",
                    &item_trait.attrs,
                );
                for trait_item in &item_trait.items {
                    if let syn::TraitItem::Fn(method) = trait_item {
                        push_item(
                            items,
                            prefix,
                            format!("{}::{}", item_trait.ident, method.sig.ident),
                            "method",
                            &method.attrs,
                        );
                    }
                }
            }
            syn::Item::Macro(item_macro)
                if has_attribute(&item_macro.attrs, "macro_export", "")
                    && !is_hidden(&item_macro.attrs) =>
            {
                if let Some(ident) = &item_macro.ident {
                    push_item(
                        items,
                        prefix,
                        format!("{ident}!"),
                        "macro",
                        &item_macro.attrs,
                    );
                }
            }
            syn::Item::Use(item_use) if is_public(&item_use.vis) && !is_hidden(&item_use.attrs) => {
                let mut uses = Vec::new();
                use_paths(&item_use.tree, String::new(), &mut uses);
                for (name, full) in uses {
                    items.push(RustItem {
                        path: format!("{prefix}{name}"),
                        kind: "re-export",
                        summary: summary(&item_use.attrs).if_empty(format!("`{full}`")),
                        note: host_only(&item_use.attrs),
                    });
                }
            }
            syn::Item::Mod(module) if is_public(&module.vis) && !is_hidden(&module.attrs) => {
                push_item(
                    items,
                    prefix,
                    module.ident.to_string(),
                    "module",
                    &module.attrs,
                );
                if let Some((_, inner)) = &module.content {
                    collect_items(inner, &format!("{prefix}{}::", module.ident), items);
                }
            }
            syn::Item::Impl(implementation)
                if implementation.trait_.is_none() && !is_hidden(&implementation.attrs) =>
            {
                let Some(owner) = type_path(&implementation.self_ty) else {
                    continue;
                };
                for impl_item in &implementation.items {
                    match impl_item {
                        syn::ImplItem::Fn(method)
                            if is_public(&method.vis) && !is_hidden(&method.attrs) =>
                        {
                            items.push(RustItem {
                                path: format!("{prefix}{owner}::{}", method.sig.ident),
                                kind: "method",
                                summary: summary(&method.attrs),
                                note: host_only(&method.attrs).or(host_only(&implementation.attrs)),
                            });
                        }
                        syn::ImplItem::Const(constant)
                            if is_public(&constant.vis) && !is_hidden(&constant.attrs) =>
                        {
                            items.push(RustItem {
                                path: format!("{prefix}{owner}::{}", constant.ident),
                                kind: "const",
                                summary: summary(&constant.attrs),
                                note: None,
                            });
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

trait IfEmpty {
    fn if_empty(self, fallback: String) -> String;
}

impl IfEmpty for String {
    fn if_empty(self, fallback: String) -> String {
        if self.is_empty() { fallback } else { self }
    }
}

fn use_paths(tree: &syn::UseTree, prefix: String, out: &mut Vec<(String, String)>) {
    match tree {
        syn::UseTree::Path(path) => use_paths(&path.tree, format!("{prefix}{}::", path.ident), out),
        syn::UseTree::Name(name) => {
            out.push((name.ident.to_string(), format!("{prefix}{}", name.ident)))
        }
        syn::UseTree::Rename(rename) => out.push((
            rename.rename.to_string(),
            format!("{prefix}{}", rename.ident),
        )),
        syn::UseTree::Glob(_) => out.push((format!("{prefix}*"), format!("{prefix}*"))),
        syn::UseTree::Group(group) => {
            for tree in &group.items {
                use_paths(tree, prefix.clone(), out);
            }
        }
    }
}

pub fn rust_page(crate_name: &str, source: &str, items: &[RustItem]) -> String {
    let mut page = header(&format!("`{crate_name}` public items"), source);
    writeln!(
        page,
        "Every public item, in path order. For signatures, run `script/fyx-api-docs` and open the \
         rustdoc it builds.\n\n| Item | Kind | Summary |\n|---|---|---|"
    )
    .ok();
    for item in items {
        let note = item.note.map_or(String::new(), |note| format!(" ({note})"));
        writeln!(
            page,
            "| `{}` | {}{note} | {} |",
            item.path,
            item.kind,
            table_cell(&item.summary)
        )
        .ok();
    }
    page
}

/// Names in a hand-written page that no longer exist: `Type::member` and `macro!` references
/// that match no Rust item, and lower-case kebab-case names (`push-update`) that match no WIT
/// name. Code blocks are skipped.
pub fn stale_references(
    markdown: &str,
    rust_items: &[RustItem],
    wit_names: &BTreeSet<String>,
) -> Vec<String> {
    let rust_paths = rust_items
        .iter()
        .map(|item| {
            item.path
                .rsplit("::")
                .take(2)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("::")
        })
        .collect::<BTreeSet<_>>();
    let owners = rust_paths
        .iter()
        .filter_map(|path| path.split_once("::").map(|(owner, _)| owner.to_string()))
        .chain(
            rust_items
                .iter()
                .filter(|item| matches!(item.kind, "struct" | "enum" | "trait"))
                .map(|item| item.path.clone()),
        )
        .collect::<BTreeSet<_>>();
    let macros = rust_items
        .iter()
        .filter(|item| item.kind == "macro")
        .map(|item| item.path.clone())
        .collect::<BTreeSet<_>>();

    let mut stale = BTreeSet::new();
    let mut in_code_block = false;
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }
        for span in line.split('`').skip(1).step_by(2) {
            let reference = span.trim_end_matches("()").trim();
            if let Some(name) = reference.strip_suffix('!')
                && is_identifier(name)
            {
                if !macros.contains(reference) {
                    stale.insert(reference.to_string());
                }
            } else if let Some((owner, member)) = reference.split_once("::")
                && is_identifier(owner)
                && is_identifier(member)
                && owners.contains(owner)
            {
                if !rust_paths.contains(reference) {
                    stale.insert(reference.to_string());
                }
            } else if is_kebab(reference)
                && !wit_names.contains(reference)
                && !NOT_WIT_NAMES.contains(&reference)
            {
                stale.insert(reference.to_string());
            }
        }
    }
    stale.into_iter().collect()
}

fn is_identifier(text: &str) -> bool {
    text.chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
        && text.chars().all(|c| c.is_alphanumeric() || c == '_')
}

fn is_kebab(text: &str) -> bool {
    text.contains('-')
        && text
            .split('-')
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase()))
}

#[cfg(test)]
mod tests;
