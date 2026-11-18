//! Which `fyx:ui` world a component targets, and the rules that differ between worlds.
//!
//! Every rule takes the version, so a 0.1 component is checked exactly as it was before 0.2
//! existed: the 0.1 arms accept everything the 0.1 host accepted, and reject only what 0.1 cannot
//! express at all (a modal, a decoration, a runtime surface).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::tree::TreeLimits;
use crate::v0_2::{
    CAPABILITY_EDITOR_DECORATIONS, DecorationLocation, HostErrorCode, Surface, SurfaceKind,
};

/// Bytes in a surface or decoration id (0.2).
pub const MAX_ID_BYTES: usize = 64;
/// Live surfaces per extension, static ones included (0.2).
pub const MAX_SURFACES: usize = 32;
/// Live decorations per extension; not counted against [`MAX_SURFACES`] (0.2).
pub const MAX_DECORATIONS: usize = 512;
/// `place-decoration` and `remove-decoration` calls per export call (0.2). The host counts them
/// in its effect list; [`LiveIds`] only knows live ids.
pub const MAX_DECORATION_EFFECTS_PER_CALL: usize = 512;
/// Bytes in a decoration path (0.2).
pub const MAX_DECORATION_PATH_BYTES: usize = 1024;
/// Indent levels the host draws; deeper list rows are clamped (0.2).
pub const MAX_INDENT_LEVELS: u8 = 32;
/// Bytes of a button tooltip the host shows; longer text is truncated (0.2).
pub const MAX_TOOLTIP_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ApiVersion {
    V0_1_0,
    V0_2_0,
}

impl ApiVersion {
    pub const ALL: [ApiVersion; 2] = [ApiVersion::V0_1_0, ApiVersion::V0_2_0];

    /// The supported set is explicit, not a semver range: for 0.x every minor version is a
    /// different world, so 0.1.1 or 0.3.0 is not "close enough" to anything.
    pub fn from_parts(major: u16, minor: u16, patch: u16) -> Option<Self> {
        match (major, minor, patch) {
            (0, 1, 0) => Some(ApiVersion::V0_1_0),
            (0, 2, 0) => Some(ApiVersion::V0_2_0),
            _ => None,
        }
    }

    pub fn parts(self) -> (u16, u16, u16) {
        match self {
            ApiVersion::V0_1_0 => (0, 1, 0),
            ApiVersion::V0_2_0 => (0, 2, 0),
        }
    }

    /// The `package` line of the version's WIT file.
    pub fn wit_package(self) -> &'static str {
        match self {
            ApiVersion::V0_1_0 => "fyx:ui@0.1.0",
            ApiVersion::V0_2_0 => "fyx:ui@0.2.0",
        }
    }

    pub fn wit_dir(self) -> &'static str {
        match self {
            ApiVersion::V0_1_0 => crate::WIT_DIR_V0_1_0,
            ApiVersion::V0_2_0 => crate::WIT_DIR_V0_2_0,
        }
    }

    /// The same for both worlds: 2048 patches per update and nodes per row, 16384 nodes per tree,
    /// which is also the cap for a `mount` and a `replace`. 0.1 only gains from the shared `mount`
    /// cap (integration-1 #11), so it needs no arm of its own.
    pub fn tree_limits(self) -> TreeLimits {
        TreeLimits::default()
    }

    /// 0.1 does not restrict surface ids. 0.2 allows 1 to [`MAX_ID_BYTES`] bytes of
    /// `[a-z0-9._-]`, so an id is safe in the host's `"<extension>/<surface>"` key and in action
    /// payloads.
    pub fn check_surface_id(self, id: &str) -> Result<(), ApiError> {
        match self {
            ApiVersion::V0_1_0 => Ok(()),
            ApiVersion::V0_2_0 => check_id(id),
        }
    }
}

impl fmt::Display for ApiVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (major, minor, patch) = self.parts();
        write!(formatter, "{major}.{minor}.{patch}")
    }
}

fn check_id(id: &str) -> Result<(), ApiError> {
    let valid = (1..=MAX_ID_BYTES).contains(&id.len())
        && id
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(ApiError::InvalidId { id: id.to_owned() })
    }
}

/// The path rule of `place-decoration` (design.md, L): relative to a worktree root,
/// `/`-separated, at most [`MAX_DECORATION_PATH_BYTES`], with no empty, `.` or `..` component.
///
/// `\` and `:` are rejected too, although the design does not list them: on Windows `\` is a
/// separator (so `..\x` would climb out of the worktree) and `C:` makes a join absolute. NUL is
/// never part of a file name.
pub fn check_decoration_path(path: &str) -> Result<(), ApiError> {
    let problem = if path.len() > MAX_DECORATION_PATH_BYTES {
        Some(PathProblem::TooLong {
            max: MAX_DECORATION_PATH_BYTES,
        })
    } else if let Some(character) = path.chars().find(|c| matches!(c, '\\' | ':' | '\0')) {
        Some(PathProblem::ForbiddenCharacter(character))
    } else {
        path.split('/').find_map(|component| match component {
            "" => Some(PathProblem::EmptyComponent),
            "." | ".." => Some(PathProblem::DotComponent),
            _ => None,
        })
    };
    match problem {
        Some(problem) => Err(ApiError::InvalidPath {
            path: path.to_owned(),
            problem,
        }),
        None => Ok(()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IdKind {
    Surface,
    Decoration,
}

impl fmt::Display for IdKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            IdKind::Surface => "surface",
            IdKind::Decoration => "decoration",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PathProblem {
    #[error("is longer than {max} bytes")]
    TooLong { max: usize },
    #[error("contains {0:?}")]
    ForbiddenCharacter(char),
    #[error("has an empty component (it must be relative, without `//`)")]
    EmptyComponent,
    #[error("has a `.` or `..` component")]
    DotComponent,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ApiError {
    #[error("{what} is not part of fyx:ui {version}")]
    NotInVersion {
        what: &'static str,
        version: ApiVersion,
    },
    #[error("id {id:?} must be 1 to {MAX_ID_BYTES} bytes of a-z, 0-9, '.', '_' and '-'")]
    InvalidId { id: String },
    #[error("id {id:?} is already a live {kind}")]
    IdInUse { id: String, kind: IdKind },
    #[error("{id:?} is a decoration, not a surface")]
    NotASurface { id: String },
    #[error("no live surface has id {id:?}")]
    UnknownSurface { id: String },
    #[error("no live decoration has id {id:?}")]
    UnknownDecoration { id: String },
    #[error("{what} needs {capability:?} in [fyx_ui].capabilities")]
    NotPermitted {
        what: String,
        capability: &'static str,
    },
    #[error("an extension has at most {max} live surfaces")]
    TooManySurfaces { max: usize },
    #[error("an extension has at most {max} live decorations")]
    TooManyDecorations { max: usize },
    #[error("a decoration tree has {count} nodes, the cap is {max}")]
    TooManyNodes { count: usize, max: usize },
    #[error("decoration path {path:?} {problem}")]
    InvalidPath { path: String, problem: PathProblem },
}

impl ApiError {
    /// The WIT `host-error-code` a 0.2 host import returns for this error.
    pub fn code(&self) -> HostErrorCode {
        match self {
            ApiError::UnknownSurface { .. } | ApiError::UnknownDecoration { .. } => {
                HostErrorCode::UnknownSurface
            }
            ApiError::TooManySurfaces { .. }
            | ApiError::TooManyDecorations { .. }
            | ApiError::TooManyNodes { .. } => HostErrorCode::LimitExceeded,
            ApiError::NotPermitted { .. } => HostErrorCode::NotPermitted,
            ApiError::NotInVersion { .. }
            | ApiError::InvalidId { .. }
            | ApiError::IdInUse { .. }
            | ApiError::NotASurface { .. }
            | ApiError::InvalidPath { .. } => HostErrorCode::InvalidArgument,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdLimits {
    pub max_surfaces: usize,
    pub max_decorations: usize,
}

impl Default for IdLimits {
    fn default() -> Self {
        Self {
            max_surfaces: MAX_SURFACES,
            max_decorations: MAX_DECORATIONS,
        }
    }
}

/// What `place-decoration` did with a valid id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    New,
    /// The id was a live decoration; it moves and its tree is replaced.
    Moved,
}

/// The live surface and decoration ids of one extension, changed as the host imports are called
/// (design.md, A.6), so a later import in the same call sees the earlier ones.
///
/// Surfaces and decorations are counted and capped separately, but share one id space: a
/// decoration id works like a surface id for `push-update`, `handle-event`, `list-rows` and
/// `mount`, so no id may be both.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveIds {
    version: ApiVersion,
    limits: IdLimits,
    surfaces: BTreeMap<String, SurfaceKind>,
    decorations: BTreeSet<String>,
}

impl LiveIds {
    /// The static surfaces from `surfaces()`. A 0.1 set is checked as the 0.1 host checks it: only
    /// that the manifest has each kind's capability. A 0.2 set goes through the `add-surface`
    /// checks one by one.
    pub fn new(
        version: ApiVersion,
        static_surfaces: &[Surface],
        capabilities: &[impl AsRef<str>],
    ) -> Result<Self, ApiError> {
        Self::with_limits(version, IdLimits::default(), static_surfaces, capabilities)
    }

    pub fn with_limits(
        version: ApiVersion,
        limits: IdLimits,
        static_surfaces: &[Surface],
        capabilities: &[impl AsRef<str>],
    ) -> Result<Self, ApiError> {
        let mut ids = Self {
            version,
            limits,
            surfaces: BTreeMap::new(),
            decorations: BTreeSet::new(),
        };
        for surface in static_surfaces {
            match version {
                ApiVersion::V0_1_0 => {
                    if matches!(surface.kind, SurfaceKind::Modal(_)) {
                        return Err(ApiError::NotInVersion {
                            what: "a modal surface",
                            version,
                        });
                    }
                    check_capability(surface, capabilities)?;
                    ids.surfaces
                        .insert(surface.id.clone(), surface.kind.clone());
                }
                ApiVersion::V0_2_0 => ids.insert_surface(surface, capabilities)?,
            }
        }
        Ok(ids)
    }

    pub fn version(&self) -> ApiVersion {
        self.version
    }

    pub fn limits(&self) -> IdLimits {
        self.limits
    }

    pub fn surface_kind(&self, id: &str) -> Option<&SurfaceKind> {
        self.surfaces.get(id)
    }

    pub fn is_decoration(&self, id: &str) -> bool {
        self.decorations.contains(id)
    }

    pub fn surfaces(&self) -> impl Iterator<Item = (&str, &SurfaceKind)> {
        self.surfaces.iter().map(|(id, kind)| (id.as_str(), kind))
    }

    pub fn decorations(&self) -> impl Iterator<Item = &str> {
        self.decorations.iter().map(String::as_str)
    }

    /// `add-surface`, checked in the design's order (E): the id rule, the id not already live,
    /// the capability, the cap.
    pub fn add_surface(
        &mut self,
        surface: &Surface,
        capabilities: &[impl AsRef<str>],
    ) -> Result<(), ApiError> {
        self.require_v0_2("add-surface")?;
        self.insert_surface(surface, capabilities)
    }

    /// `remove-surface`. Returns the kind the surface had.
    pub fn remove_surface(&mut self, id: &str) -> Result<SurfaceKind, ApiError> {
        self.require_v0_2("remove-surface")?;
        if self.decorations.contains(id) {
            return Err(ApiError::NotASurface { id: id.to_owned() });
        }
        self.surfaces
            .remove(id)
            .ok_or_else(|| ApiError::UnknownSurface { id: id.to_owned() })
    }

    /// `place-decoration`, checked in the design's order (L): the capability, the id rule, the
    /// id not a live surface, the path, the node count, and for a new id the cap. Whether the
    /// tree fits is checked when it is applied, as for pushes.
    pub fn place_decoration(
        &mut self,
        id: &str,
        location: &DecorationLocation,
        node_count: usize,
        capabilities: &[impl AsRef<str>],
    ) -> Result<Placement, ApiError> {
        self.require_v0_2("place-decoration")?;
        check_decorations_permitted(capabilities)?;
        check_id(id)?;
        if self.surfaces.contains_key(id) {
            return Err(ApiError::IdInUse {
                id: id.to_owned(),
                kind: IdKind::Surface,
            });
        }
        check_decoration_path(&location.path)?;
        let max_nodes = self.version.tree_limits().max_nodes;
        if node_count > max_nodes {
            return Err(ApiError::TooManyNodes {
                count: node_count,
                max: max_nodes,
            });
        }
        if self.decorations.contains(id) {
            return Ok(Placement::Moved);
        }
        if self.decorations.len() >= self.limits.max_decorations {
            return Err(ApiError::TooManyDecorations {
                max: self.limits.max_decorations,
            });
        }
        self.decorations.insert(id.to_owned());
        Ok(Placement::New)
    }

    /// `remove-decoration`.
    pub fn remove_decoration(
        &mut self,
        id: &str,
        capabilities: &[impl AsRef<str>],
    ) -> Result<(), ApiError> {
        self.require_v0_2("remove-decoration")?;
        check_decorations_permitted(capabilities)?;
        if self.decorations.remove(id) {
            Ok(())
        } else {
            Err(ApiError::UnknownDecoration { id: id.to_owned() })
        }
    }

    fn require_v0_2(&self, what: &'static str) -> Result<(), ApiError> {
        match self.version {
            ApiVersion::V0_1_0 => Err(ApiError::NotInVersion {
                what,
                version: self.version,
            }),
            ApiVersion::V0_2_0 => Ok(()),
        }
    }

    fn insert_surface(
        &mut self,
        surface: &Surface,
        capabilities: &[impl AsRef<str>],
    ) -> Result<(), ApiError> {
        check_id(&surface.id)?;
        let in_use = if self.surfaces.contains_key(&surface.id) {
            Some(IdKind::Surface)
        } else if self.decorations.contains(&surface.id) {
            Some(IdKind::Decoration)
        } else {
            None
        };
        if let Some(kind) = in_use {
            return Err(ApiError::IdInUse {
                id: surface.id.clone(),
                kind,
            });
        }
        check_capability(surface, capabilities)?;
        if self.surfaces.len() >= self.limits.max_surfaces {
            return Err(ApiError::TooManySurfaces {
                max: self.limits.max_surfaces,
            });
        }
        self.surfaces
            .insert(surface.id.clone(), surface.kind.clone());
        Ok(())
    }
}

fn has_capability(capabilities: &[impl AsRef<str>], required: &str) -> bool {
    capabilities
        .iter()
        .any(|capability| capability.as_ref() == required)
}

fn check_capability(surface: &Surface, capabilities: &[impl AsRef<str>]) -> Result<(), ApiError> {
    let capability = surface.kind.capability();
    if has_capability(capabilities, capability) {
        Ok(())
    } else {
        Err(ApiError::NotPermitted {
            what: format!("surface {:?}", surface.id),
            capability,
        })
    }
}

fn check_decorations_permitted(capabilities: &[impl AsRef<str>]) -> Result<(), ApiError> {
    if has_capability(capabilities, CAPABILITY_EDITOR_DECORATIONS) {
        Ok(())
    } else {
        Err(ApiError::NotPermitted {
            what: "editor decorations".to_owned(),
            capability: CAPABILITY_EDITOR_DECORATIONS,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v0_2::{
        DecorationPlacement, DockPosition, Icon, ModalDescriptor, ModalWidth, PanelDescriptor,
        StatusBarSide, StatusItemDescriptor,
    };

    const PANEL: &str = "ui:panel";
    const STATUS: &str = "ui:status-bar";
    const MODAL: &str = "ui:modal";
    const DECORATIONS: &str = "ui:editor-decorations";
    const ALL_CAPABILITIES: [&str; 4] = [PANEL, STATUS, MODAL, DECORATIONS];

    fn panel(id: &str) -> Surface {
        Surface {
            id: id.into(),
            title: id.into(),
            kind: SurfaceKind::Panel(PanelDescriptor {
                position: DockPosition::Left,
                icon: Icon::Blocks.into(),
                default_size: 240,
            }),
        }
    }

    fn status(id: &str) -> Surface {
        Surface {
            id: id.into(),
            title: id.into(),
            kind: SurfaceKind::StatusItem(StatusItemDescriptor {
                side: StatusBarSide::Right,
                priority: 0,
            }),
        }
    }

    fn modal(id: &str) -> Surface {
        Surface {
            id: id.into(),
            title: id.into(),
            kind: SurfaceKind::Modal(ModalDescriptor {
                width: ModalWidth::Medium,
            }),
        }
    }

    fn at(path: &str) -> DecorationLocation {
        DecorationLocation {
            path: path.into(),
            line: 0,
            placement: DecorationPlacement::Above,
            height_lines: None,
        }
    }

    fn v0_2(static_surfaces: &[Surface]) -> LiveIds {
        LiveIds::new(ApiVersion::V0_2_0, static_surfaces, &ALL_CAPABILITIES).expect("valid set")
    }

    #[test]
    fn versions_are_an_explicit_set() {
        for version in ApiVersion::ALL {
            let (major, minor, patch) = version.parts();
            assert_eq!(ApiVersion::from_parts(major, minor, patch), Some(version));
            assert_eq!(version.wit_package(), format!("fyx:ui@{version}"));
        }
        for (major, minor, patch) in [(0, 0, 1), (0, 1, 1), (0, 3, 0), (1, 0, 0), (0, 2, 1)] {
            assert_eq!(ApiVersion::from_parts(major, minor, patch), None);
        }
        assert!(ApiVersion::V0_1_0 < ApiVersion::V0_2_0);
        assert_eq!(ApiVersion::V0_2_0.to_string(), "0.2.0");
    }

    #[test]
    fn each_version_points_at_its_wit_package() {
        let packages = [
            (ApiVersion::V0_1_0, include_str!("../wit/fyx-ui.wit")),
            (
                ApiVersion::V0_2_0,
                include_str!("../wit/since_v0.2.0/fyx-ui.wit"),
            ),
        ];
        for (version, wit) in packages {
            let package = format!("package {};", version.wit_package());
            assert!(
                wit.lines().any(|line| line == package),
                "the {version} WIT lacks {package:?}"
            );
            let file = std::path::Path::new(version.wit_dir()).join("fyx-ui.wit");
            assert_eq!(std::fs::read_to_string(&file).ok().as_deref(), Some(wit));
        }
    }

    #[test]
    fn mount_and_replace_cap_is_the_tree_cap_in_both_versions() {
        for version in ApiVersion::ALL {
            let limits = version.tree_limits();
            assert_eq!(limits.max_nodes, 16384);
            assert_eq!(limits.max_nodes_per_update, 2048);
        }
    }

    #[test]
    fn surface_id_rule() {
        for id in ["counter", "a", "gallery-status", "x.y_z-0", &"a".repeat(64)] {
            assert_eq!(ApiVersion::V0_2_0.check_surface_id(id), Ok(()), "{id:?}");
        }
        for id in [
            "",
            "Counter",
            "with space",
            "a/b",
            "ünï",
            "tab\t",
            &"a".repeat(65),
        ] {
            assert_eq!(
                ApiVersion::V0_2_0.check_surface_id(id),
                Err(ApiError::InvalidId { id: id.into() }),
            );
            assert_eq!(ApiVersion::V0_1_0.check_surface_id(id), Ok(()));
        }
    }

    #[test]
    fn decoration_path_rule() {
        for path in [
            "src/main.rs",
            "a",
            "dir/.hidden",
            "x..y/z",
            &"a".repeat(1024),
        ] {
            assert_eq!(check_decoration_path(path), Ok(()), "{path:?}");
        }
        let cases = [
            ("", PathProblem::EmptyComponent),
            ("/etc/passwd", PathProblem::EmptyComponent),
            ("src//main.rs", PathProblem::EmptyComponent),
            ("src/", PathProblem::EmptyComponent),
            ("./src", PathProblem::DotComponent),
            ("src/../../x", PathProblem::DotComponent),
            ("..", PathProblem::DotComponent),
            ("..\\..\\x", PathProblem::ForbiddenCharacter('\\')),
            ("C:/Windows", PathProblem::ForbiddenCharacter(':')),
            ("a\0b", PathProblem::ForbiddenCharacter('\0')),
            (&"a".repeat(1025), PathProblem::TooLong { max: 1024 }),
        ];
        for (path, problem) in cases {
            assert_eq!(
                check_decoration_path(path),
                Err(ApiError::InvalidPath {
                    path: path.into(),
                    problem
                }),
            );
        }
    }

    #[test]
    fn v0_1_static_surfaces_are_checked_only_for_capabilities() {
        // Ids that 0.2 rejects, and a duplicate, as the 0.1 host accepts them.
        let surfaces = [panel("Counter Panel"), panel("Counter Panel"), status("")];
        let ids = LiveIds::new(ApiVersion::V0_1_0, &surfaces, &[PANEL, STATUS]).expect("valid");
        assert!(ids.surface_kind("Counter Panel").is_some());
        assert_eq!(
            LiveIds::new(ApiVersion::V0_1_0, &surfaces, &[PANEL]),
            Err(ApiError::NotPermitted {
                what: "surface \"\"".into(),
                capability: STATUS,
            })
        );
        let error = LiveIds::new(ApiVersion::V0_1_0, &[modal("m")], &ALL_CAPABILITIES).unwrap_err();
        assert!(matches!(error, ApiError::NotInVersion { .. }));

        let many: Vec<Surface> = (0..100).map(|index| panel(&format!("p{index}"))).collect();
        LiveIds::new(ApiVersion::V0_1_0, &many, &[PANEL]).expect("0.1 has no surface cap");
    }

    #[test]
    fn v0_1_has_no_runtime_surfaces_or_decorations() {
        let mut ids =
            LiveIds::new(ApiVersion::V0_1_0, &[panel("p")], &ALL_CAPABILITIES).expect("valid");
        let before = ids.clone();
        let not_in_v0_1 = |result: Result<(), ApiError>| {
            assert!(matches!(
                result,
                Err(ApiError::NotInVersion {
                    version: ApiVersion::V0_1_0,
                    ..
                })
            ));
        };
        not_in_v0_1(ids.add_surface(&panel("q"), &ALL_CAPABILITIES));
        not_in_v0_1(ids.remove_surface("p").map(drop));
        not_in_v0_1(
            ids.place_decoration("d", &at("a.rs"), 1, &ALL_CAPABILITIES)
                .map(drop),
        );
        not_in_v0_1(ids.remove_decoration("d", &ALL_CAPABILITIES));
        assert_eq!(ids, before);
    }

    #[test]
    fn v0_2_add_surface_checks_in_order() {
        let mut ids = v0_2(&[panel("counter")]);
        // The id rule comes before the capability check.
        assert_eq!(
            ids.add_surface(&modal("Bad Id"), &[PANEL]),
            Err(ApiError::InvalidId {
                id: "Bad Id".into()
            })
        );
        assert_eq!(
            ids.add_surface(&modal("counter"), &[PANEL]),
            Err(ApiError::IdInUse {
                id: "counter".into(),
                kind: IdKind::Surface
            })
        );
        assert_eq!(
            ids.add_surface(&modal("picker"), &[PANEL]),
            Err(ApiError::NotPermitted {
                what: "surface \"picker\"".into(),
                capability: MODAL
            })
        );
        ids.add_surface(&modal("picker"), &ALL_CAPABILITIES)
            .expect("valid");
        assert!(matches!(
            ids.surface_kind("picker"),
            Some(SurfaceKind::Modal(_))
        ));
        assert!(matches!(
            ids.remove_surface("picker"),
            Ok(SurfaceKind::Modal(_))
        ));
        assert_eq!(
            ids.remove_surface("picker"),
            Err(ApiError::UnknownSurface {
                id: "picker".into()
            })
        );
        // The id is free again.
        ids.add_surface(&status("picker"), &ALL_CAPABILITIES)
            .expect("valid");
    }

    #[test]
    fn v0_2_surface_cap_counts_static_surfaces() {
        let statics: Vec<Surface> = (0..MAX_SURFACES - 1)
            .map(|index| panel(&format!("p{index}")))
            .collect();
        let mut ids = v0_2(&statics);
        ids.add_surface(&status("last"), &ALL_CAPABILITIES)
            .expect("the 32nd surface fits");
        assert_eq!(
            ids.add_surface(&status("one-more"), &ALL_CAPABILITIES),
            Err(ApiError::TooManySurfaces { max: 32 })
        );
        ids.remove_surface("p0").expect("live");
        ids.add_surface(&status("one-more"), &ALL_CAPABILITIES)
            .expect("room again");

        let too_many: Vec<Surface> = (0..=MAX_SURFACES)
            .map(|index| panel(&format!("p{index}")))
            .collect();
        assert_eq!(
            LiveIds::new(ApiVersion::V0_2_0, &too_many, &ALL_CAPABILITIES),
            Err(ApiError::TooManySurfaces { max: 32 })
        );
        assert_eq!(
            LiveIds::new(
                ApiVersion::V0_2_0,
                &[panel("a"), status("a")],
                &ALL_CAPABILITIES
            ),
            Err(ApiError::IdInUse {
                id: "a".into(),
                kind: IdKind::Surface
            })
        );
    }

    #[test]
    fn decorations_check_capability_first() {
        let mut ids = v0_2(&[panel("counter")]);
        let not_permitted = Err(ApiError::NotPermitted {
            what: "editor decorations".into(),
            capability: DECORATIONS,
        });
        assert_eq!(
            ids.place_decoration("Bad Id", &at("/abs"), usize::MAX, &[PANEL]),
            not_permitted
        );
        assert_eq!(
            ids.remove_decoration("missing", &[PANEL]),
            not_permitted.map(drop)
        );
    }

    #[test]
    fn decorations_share_the_id_space_but_not_the_cap() {
        let mut ids = v0_2(&[panel("counter")]);
        assert_eq!(
            ids.place_decoration("counter", &at("a.rs"), 1, &ALL_CAPABILITIES),
            Err(ApiError::IdInUse {
                id: "counter".into(),
                kind: IdKind::Surface
            })
        );
        assert_eq!(
            ids.place_decoration("Todo 1", &at("a.rs"), 1, &ALL_CAPABILITIES),
            Err(ApiError::InvalidId {
                id: "Todo 1".into()
            })
        );
        assert!(matches!(
            ids.place_decoration("todo-1", &at("../a.rs"), 1, &ALL_CAPABILITIES),
            Err(ApiError::InvalidPath { .. })
        ));
        assert_eq!(
            ids.place_decoration("todo-1", &at("a.rs"), 16385, &ALL_CAPABILITIES),
            Err(ApiError::TooManyNodes {
                count: 16385,
                max: 16384
            })
        );
        assert_eq!(
            ids.place_decoration("todo-1", &at("a.rs"), 16384, &ALL_CAPABILITIES),
            Ok(Placement::New)
        );
        assert_eq!(
            ids.place_decoration("todo-1", &at("b.rs"), 1, &ALL_CAPABILITIES),
            Ok(Placement::Moved)
        );
        assert!(ids.is_decoration("todo-1"));
        assert_eq!(
            ids.add_surface(&panel("todo-1"), &ALL_CAPABILITIES),
            Err(ApiError::IdInUse {
                id: "todo-1".into(),
                kind: IdKind::Decoration
            })
        );
        assert_eq!(
            ids.remove_surface("todo-1"),
            Err(ApiError::NotASurface {
                id: "todo-1".into()
            })
        );

        // 512 decorations on top of the surfaces; a live id can still move at the cap.
        for index in 1..MAX_DECORATIONS {
            ids.place_decoration(&format!("d{index}"), &at("a.rs"), 1, &ALL_CAPABILITIES)
                .expect("under the cap");
        }
        assert_eq!(ids.decorations().count(), MAX_DECORATIONS);
        assert_eq!(
            ids.place_decoration("d-extra", &at("a.rs"), 1, &ALL_CAPABILITIES),
            Err(ApiError::TooManyDecorations { max: 512 })
        );
        assert_eq!(
            ids.place_decoration("d1", &at("c.rs"), 1, &ALL_CAPABILITIES),
            Ok(Placement::Moved)
        );
        ids.add_surface(&status("still-room"), &ALL_CAPABILITIES)
            .expect("decorations do not count against surfaces");

        ids.remove_decoration("d1", &ALL_CAPABILITIES)
            .expect("live");
        assert_eq!(
            ids.remove_decoration("d1", &ALL_CAPABILITIES),
            Err(ApiError::UnknownDecoration { id: "d1".into() })
        );
        assert_eq!(
            ids.place_decoration("d-extra", &at("a.rs"), 1, &ALL_CAPABILITIES),
            Ok(Placement::New)
        );
    }

    #[test]
    fn errors_map_to_host_error_codes() {
        let cases = [
            (
                ApiError::UnknownSurface { id: "x".into() },
                HostErrorCode::UnknownSurface,
            ),
            (
                ApiError::UnknownDecoration { id: "x".into() },
                HostErrorCode::UnknownSurface,
            ),
            (
                ApiError::TooManySurfaces { max: 32 },
                HostErrorCode::LimitExceeded,
            ),
            (
                ApiError::TooManyDecorations { max: 512 },
                HostErrorCode::LimitExceeded,
            ),
            (
                ApiError::TooManyNodes { count: 2, max: 1 },
                HostErrorCode::LimitExceeded,
            ),
            (
                ApiError::NotPermitted {
                    what: "x".into(),
                    capability: MODAL,
                },
                HostErrorCode::NotPermitted,
            ),
            (
                ApiError::InvalidId { id: "X".into() },
                HostErrorCode::InvalidArgument,
            ),
            (
                ApiError::NotASurface { id: "x".into() },
                HostErrorCode::InvalidArgument,
            ),
        ];
        for (error, code) in cases {
            assert_eq!(error.code(), code, "{error}");
            let host_error = crate::v0_2::HostError::from(&error);
            assert_eq!(host_error.code, code);
            assert_eq!(host_error.message, error.to_string());
        }
    }
}
