//! Patch persistence: snapshots of every host parameter saved as versioned
//! JSON files in a per-user patch library.
//!
//! A patch stores plain (unnormalized) values keyed by the stable parameter
//! IDs, plus each parameter's name for readability. Loading tolerates older
//! and newer files: parameters missing from a file fall back to their
//! default, and unknown IDs are ignored. Node-edited LFO shapes are stored
//! alongside the parameters; files without them play the Shape presets.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use truce::params::{ParamFlags, ParamInfo, Params};

use crate::engine::{LfoPoint, LfoShape, MAX_LFOS};
use crate::plugin::CustomLfoShapes;

/// Name shown for the built-in initial patch. Reserved: user patches may not
/// use it.
pub const DEFAULT_PATCH_NAME: &str = "Default";
pub const PATCH_EXTENSION: &str = "synthol";
pub const MAX_PATCH_NAME_LEN: usize = 64;

const PATCH_FORMAT: &str = "synthol-patch";
const PATCH_VERSION: u32 = 1;

#[derive(Debug, PartialEq, Eq)]
pub enum PatchError {
    InvalidName(&'static str),
    NoLibraryDirectory,
    NotFound(String),
    Io(String),
    Format(String),
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName(reason) => f.write_str(reason),
            Self::NoLibraryDirectory => f.write_str("Could not locate the patch folder."),
            Self::NotFound(name) => write!(f, "Patch \"{name}\" was not found."),
            Self::Io(message) => write!(f, "Could not access the patch file: {message}"),
            Self::Format(message) => write!(f, "The patch file is invalid: {message}"),
        }
    }
}

impl From<io::Error> for PatchError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

/// Validate a user-entered patch name and return it trimmed.
pub fn validate_name(raw: &str) -> Result<String, PatchError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(PatchError::InvalidName("Enter a patch name."));
    }
    if name.eq_ignore_ascii_case(DEFAULT_PATCH_NAME) {
        return Err(PatchError::InvalidName("\"Default\" is reserved."));
    }
    if name.chars().count() > MAX_PATCH_NAME_LEN {
        return Err(PatchError::InvalidName("Name is too long (max 64)."));
    }
    if name.starts_with('.') {
        return Err(PatchError::InvalidName("Name can't start with \".\"."));
    }
    if name.chars().any(|c| {
        c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
    }) {
        return Err(PatchError::InvalidName(
            "Name can't contain / \\ : * ? \" < > |",
        ));
    }
    Ok(name.to_owned())
}

#[must_use]
pub fn is_default_name(name: &str) -> bool {
    name.is_empty() || name == DEFAULT_PATCH_NAME
}

fn is_patch_param(info: &ParamInfo) -> bool {
    !info.flags.contains(ParamFlags::READONLY)
}

/// A snapshot of plain parameter values keyed by parameter ID, plus the
/// custom LFO shapes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Patch {
    values: HashMap<u32, f64>,
    lfo_shapes: CustomLfoShapes,
}

impl Patch {
    /// The built-in "Default" patch: every parameter at its default.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn capture<P: Params + ?Sized>(params: &P) -> Self {
        let values = params
            .param_infos()
            .iter()
            .filter(|info| is_patch_param(info))
            .filter_map(|info| params.get_plain(info.id).map(|value| (info.id, value)))
            .collect();
        Self {
            values,
            lfo_shapes: CustomLfoShapes::default(),
        }
    }

    #[must_use]
    pub fn with_lfo_shapes(mut self, lfo_shapes: CustomLfoShapes) -> Self {
        self.lfo_shapes = lfo_shapes;
        self
    }

    #[must_use]
    pub fn lfo_shapes(&self) -> CustomLfoShapes {
        self.lfo_shapes
    }

    /// Normalized target value for every patchable parameter. Parameters the
    /// patch doesn't mention resolve to their default.
    #[must_use]
    pub fn normalized_values<P: Params + ?Sized>(&self, params: &P) -> Vec<(u32, f64)> {
        params
            .param_infos()
            .iter()
            .filter(|info| is_patch_param(info))
            .map(|info| {
                let plain = self
                    .values
                    .get(&info.id)
                    .copied()
                    .filter(|value| value.is_finite())
                    .unwrap_or(info.default_plain);
                (info.id, info.range.normalize(plain).clamp(0.0, 1.0))
            })
            .collect()
    }

    pub fn to_json<P: Params + ?Sized>(
        &self,
        name: &str,
        params: &P,
    ) -> Result<String, PatchError> {
        let parameters = params
            .param_infos()
            .iter()
            .filter_map(|info| {
                self.values.get(&info.id).map(|&value| PatchFileParam {
                    id: info.id,
                    name: info.name.to_owned(),
                    value,
                })
            })
            .collect();
        let file = PatchFile {
            format: PATCH_FORMAT.to_owned(),
            version: PATCH_VERSION,
            name: name.to_owned(),
            parameters,
            lfo_shapes: self
                .lfo_shapes
                .0
                .iter()
                .enumerate()
                .filter_map(|(index, shape)| {
                    shape.map(|shape| PatchFileLfoShape {
                        index,
                        smooth: shape.is_smooth(),
                        points: shape
                            .points()
                            .iter()
                            .map(|point| [point.x, point.y, point.curve])
                            .collect(),
                    })
                })
                .collect(),
        };
        serde_json::to_string_pretty(&file).map_err(|error| PatchError::Format(error.to_string()))
    }

    pub fn from_json(json: &str) -> Result<Self, PatchError> {
        let file: PatchFile =
            serde_json::from_str(json).map_err(|error| PatchError::Format(error.to_string()))?;
        if file.format != PATCH_FORMAT {
            return Err(PatchError::Format("not a Synthol patch".to_owned()));
        }
        if file.version > PATCH_VERSION {
            return Err(PatchError::Format(format!(
                "version {} is newer than this Synthol supports",
                file.version
            )));
        }
        let mut lfo_shapes = CustomLfoShapes::default();
        for shape in file
            .lfo_shapes
            .into_iter()
            .filter(|shape| shape.index < MAX_LFOS)
        {
            let points: Vec<_> = shape
                .points
                .iter()
                .map(|&[x, y, curve]| LfoPoint::curved(x, y, curve))
                .collect();
            lfo_shapes.0[shape.index] = Some(LfoShape::new(&points, shape.smooth));
        }
        Ok(Self {
            values: file
                .parameters
                .into_iter()
                .map(|param| (param.id, param.value))
                .collect(),
            lfo_shapes,
        })
    }
}

#[derive(Serialize, Deserialize)]
struct PatchFile {
    format: String,
    version: u32,
    #[serde(default)]
    name: String,
    parameters: Vec<PatchFileParam>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    lfo_shapes: Vec<PatchFileLfoShape>,
}

/// A node-edited LFO shape; `points` are `[x, y, curve]`.
#[derive(Serialize, Deserialize)]
struct PatchFileLfoShape {
    index: usize,
    #[serde(default)]
    smooth: bool,
    points: Vec<[f32; 3]>,
}

#[derive(Serialize, Deserialize)]
struct PatchFileParam {
    id: u32,
    #[serde(default)]
    name: String,
    value: f64,
}

/// The on-disk folder of `.synthol` patch files. Display names are file stems.
#[derive(Clone, Debug)]
pub struct PatchLibrary {
    dir: Option<PathBuf>,
}

impl PatchLibrary {
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: Some(dir.into()),
        }
    }

    /// The per-user library: `~/Library/Application Support/Synthol/Patches`
    /// on macOS, `%APPDATA%\Synthol\Patches` on Windows and
    /// `$XDG_DATA_HOME/synthol/patches` (or `~/.local/share/...`) elsewhere.
    #[must_use]
    pub fn user() -> Self {
        Self {
            dir: user_patch_dir(),
        }
    }

    #[must_use]
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    fn require_dir(&self) -> Result<&Path, PatchError> {
        self.dir.as_deref().ok_or(PatchError::NoLibraryDirectory)
    }

    fn path_for(dir: &Path, name: &str) -> PathBuf {
        dir.join(format!("{name}.{PATCH_EXTENSION}"))
    }

    /// Saved patch names sorted case-insensitively. A missing folder is an
    /// empty library.
    #[must_use]
    pub fn list(&self) -> Vec<String> {
        let Some(Ok(entries)) = self.dir.as_deref().map(fs::read_dir) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension()?.to_str()? != PATCH_EXTENSION {
                    return None;
                }
                let stem = path.file_stem()?.to_str()?;
                validate_name(stem).ok().filter(|name| name == stem)
            })
            .collect();
        names.sort_by(|a, b| {
            a.to_lowercase()
                .cmp(&b.to_lowercase())
                .then_with(|| a.cmp(b))
        });
        names
    }

    /// The stored spelling of `name`, matched case-insensitively.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<String> {
        let wanted = name.to_lowercase();
        self.list()
            .into_iter()
            .find(|existing| existing.to_lowercase() == wanted)
    }

    pub fn load(&self, name: &str) -> Result<Patch, PatchError> {
        let dir = self.require_dir()?;
        let path = Self::path_for(dir, name);
        let json = fs::read_to_string(&path).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                PatchError::NotFound(name.to_owned())
            } else {
                error.into()
            }
        })?;
        Patch::from_json(&json)
    }

    /// Write `patch` as `name`, replacing any patch whose name matches
    /// case-insensitively. Returns the validated name.
    pub fn save<P: Params + ?Sized>(
        &self,
        name: &str,
        patch: &Patch,
        params: &P,
    ) -> Result<String, PatchError> {
        let name = validate_name(name)?;
        let dir = self.require_dir()?;
        fs::create_dir_all(dir)?;
        let json = patch.to_json(&name, params)?;
        let temp = dir.join(format!(".{name}.{PATCH_EXTENSION}.tmp"));
        fs::write(&temp, json)?;
        if let Some(existing) = self.find(&name).filter(|existing| *existing != name) {
            // Renaming case only: drop the old spelling first so
            // case-insensitive file systems pick up the new one.
            if let Err(error) = fs::remove_file(Self::path_for(dir, &existing)) {
                let _ = fs::remove_file(&temp);
                return Err(error.into());
            }
        }
        if let Err(error) = fs::rename(&temp, Self::path_for(dir, &name)) {
            let _ = fs::remove_file(&temp);
            return Err(error.into());
        }
        Ok(name)
    }

    pub fn delete(&self, name: &str) -> Result<(), PatchError> {
        let dir = self.require_dir()?;
        fs::remove_file(Self::path_for(dir, name)).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                PatchError::NotFound(name.to_owned())
            } else {
                error.into()
            }
        })
    }
}

fn non_empty_env(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn user_patch_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        Some(non_empty_env("HOME")?.join("Library/Application Support/Synthol/Patches"))
    } else if cfg!(target_os = "windows") {
        Some(non_empty_env("APPDATA")?.join("Synthol").join("Patches"))
    } else {
        let data = non_empty_env("XDG_DATA_HOME")
            .or_else(|| non_empty_env("HOME").map(|home| home.join(".local/share")))?;
        Some(data.join("synthol").join("patches"))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::plugin::{SynthParams, SynthParamsParamId};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A fresh, empty directory under the system temp dir, removed on drop.
    pub(crate) struct TempDir(pub PathBuf);

    impl TempDir {
        pub(crate) fn new(label: &str) -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "synthol-{label}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&dir);
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn apply(params: &SynthParams, patch: &Patch) {
        for (id, value) in patch.normalized_values(params) {
            params.set_normalized(id, value);
        }
    }

    #[test]
    fn validates_names() {
        assert_eq!(validate_name("  Warm Pad  "), Ok("Warm Pad".to_owned()));
        assert_eq!(
            validate_name("Bass #2 (dark)"),
            Ok("Bass #2 (dark)".to_owned())
        );
        for bad in [
            "", "   ", "Default", "default", ".hidden", "a/b", "a\\b", "a:b", "a?", "a*", "a|b",
            "a\"b", "<a>", "a\tb",
        ] {
            assert!(validate_name(bad).is_err(), "{bad:?} should be rejected");
        }
        assert!(validate_name(&"x".repeat(MAX_PATCH_NAME_LEN)).is_ok());
        assert!(validate_name(&"x".repeat(MAX_PATCH_NAME_LEN + 1)).is_err());
    }

    #[test]
    fn capture_and_apply_round_trips_through_json() {
        let source = SynthParams::default();
        source.set_normalized(SynthParamsParamId::FilterCutoff.into(), 0.25);
        source.set_normalized(SynthParamsParamId::OscCount.into(), 1.0);
        source.set_normalized(SynthParamsParamId::Osc2Level.into(), 0.4);
        let json = Patch::capture(&source).to_json("Test", &source).unwrap();
        assert!(json.contains("\"format\": \"synthol-patch\""));

        let target = SynthParams::default();
        apply(&target, &Patch::from_json(&json).unwrap());
        for info in source.param_infos() {
            let expected = source.get_normalized(info.id).unwrap();
            let actual = target.get_normalized(info.id).unwrap();
            assert!((expected - actual).abs() < 1e-9, "{} differs", info.name);
        }
    }

    #[test]
    fn custom_lfo_shapes_round_trip_through_json() {
        use crate::engine::{LfoPoint, LfoShape};
        let params = SynthParams::default();
        let plain = Patch::capture(&params).to_json("Plain", &params).unwrap();
        assert!(!plain.contains("lfo_shapes"));
        assert_eq!(
            Patch::from_json(&plain).unwrap().lfo_shapes(),
            CustomLfoShapes::default()
        );

        let shape = LfoShape::new(
            &[LfoPoint::curved(0.1, -0.5, -2.0), LfoPoint::new(0.6, 1.0)],
            true,
        );
        let mut shapes = CustomLfoShapes::default();
        shapes.0[2] = Some(shape);
        let json = Patch::capture(&params)
            .with_lfo_shapes(shapes)
            .to_json("Shaped", &params)
            .unwrap();
        assert_eq!(Patch::from_json(&json).unwrap().lfo_shapes(), shapes);

        let foreign = r#"{"format":"synthol-patch","version":1,"parameters":[],
            "lfo_shapes":[{"index":99,"smooth":false,"points":[[0.0,1.0,0.0]]}]}"#;
        assert_eq!(
            Patch::from_json(foreign).unwrap().lfo_shapes(),
            CustomLfoShapes::default()
        );
    }

    #[test]
    fn missing_parameters_use_defaults_and_unknown_ids_are_ignored() {
        let cutoff: u32 = SynthParamsParamId::FilterCutoff.into();
        let json = format!(
            r#"{{"format":"synthol-patch","version":1,"parameters":[{{"id":{cutoff},"value":1000.0}},{{"id":4242424242,"value":3.0}}]}}"#
        );
        let patch = Patch::from_json(&json).unwrap();
        let params = SynthParams::default();
        params.set_normalized(SynthParamsParamId::FilterQ.into(), 0.9);
        apply(&params, &patch);
        assert!((params.get_plain(cutoff).unwrap() - 1000.0).abs() < 1e-6);
        let resonance = params
            .param_infos()
            .into_iter()
            .find(|info| info.id == u32::from(SynthParamsParamId::FilterQ))
            .unwrap();
        assert!((params.get_plain(resonance.id).unwrap() - resonance.default_plain).abs() < 1e-9);
    }

    #[test]
    fn rejects_foreign_and_future_files() {
        assert!(Patch::from_json("not json").is_err());
        assert!(Patch::from_json(r#"{"format":"other","version":1,"parameters":[]}"#).is_err());
        assert!(
            Patch::from_json(r#"{"format":"synthol-patch","version":99,"parameters":[]}"#).is_err()
        );
    }

    #[test]
    fn library_saves_lists_loads_and_deletes() {
        let temp = TempDir::new("library");
        let library = PatchLibrary::new(&temp.0);
        assert!(library.list().is_empty());

        let params = SynthParams::default();
        params.set_normalized(SynthParamsParamId::FilterCutoff.into(), 0.3);
        let patch = Patch::capture(&params);
        assert_eq!(
            library.save(" zeta ", &patch, &params),
            Ok("zeta".to_owned())
        );
        library.save("Alpha", &patch, &params).unwrap();
        library.save("beta", &patch, &params).unwrap();
        fs::write(temp.0.join("notes.txt"), "ignored").unwrap();
        assert_eq!(library.list(), ["Alpha", "beta", "zeta"]);
        assert_eq!(library.find("ALPHA"), Some("Alpha".to_owned()));

        assert_eq!(library.load("beta").unwrap(), patch);
        assert!(matches!(
            library.load("missing"),
            Err(PatchError::NotFound(_))
        ));
        assert!(library.save("Default", &patch, &params).is_err());

        // Re-saving with different case replaces rather than duplicates.
        library.save("ALPHA", &patch, &params).unwrap();
        assert_eq!(library.list(), ["ALPHA", "beta", "zeta"]);

        library.delete("beta").unwrap();
        assert_eq!(library.list(), ["ALPHA", "zeta"]);
        assert!(library.delete("beta").is_err());
    }
}
