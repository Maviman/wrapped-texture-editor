// Owner: core-persist (C6). Manifest version handling of ARCHITECTURE.md 7.3: the migration chain and the two-stage
// parse (version fields first, then the typed manifest) shared by `load`, `read_manifest` and `extract_sources`.

use serde_json::Value;

use crate::error::ProjectError;
use crate::project::FORMAT_VERSION;
use crate::project::manifest::Manifest;

/// One migration step: a pure function from the manifest of version N to the manifest of version N + 1.
type Step = fn(Value) -> Result<Value, ProjectError>;

/// `STEPS[i]` upgrades a manifest of format version `i + 1` to version `i + 2`; a format change appends its
/// `vN_to_vN1` here and bumps `FORMAT_VERSION`.
const STEPS: &[Step] = &[v1_to_v2, v2_to_v3];

/// Version 1 -> 2 (round 4, FEATURE_TEXT 4.3): a text entry gained the optional `fx` object. A version-1 text has
/// none, and serde fills in the default, so the manifest is returned unchanged.
fn v1_to_v2(manifest: Value) -> Result<Value, ProjectError> {
    Ok(manifest)
}

/// Version 2 -> 3 (round 6): a projected placement gained the optional `mode`, `wrap_gaps` and `wrap_overlap` keys.
/// A version-2 placement has none (= Project, options off), and serde fills in the defaults, so the manifest is
/// returned unchanged.
fn v2_to_v3(manifest: Value) -> Result<Value, ProjectError> {
    Ok(manifest)
}

/// Upgrades a manifest of format version `from_version` to `FORMAT_VERSION` (a chain of pure `vN_to_vN1` steps).
///
/// A manifest that is already at `FORMAT_VERSION` or newer is returned unchanged (a newer one is read with the
/// fields this build knows). Version 0 never existed and is `ManifestInvalid`. After an upgrade the value's
/// `format_version` is `FORMAT_VERSION`.
pub fn upgrade(manifest: Value, from_version: u32) -> Result<Value, ProjectError> {
    if from_version == 0 {
        return Err(ProjectError::ManifestInvalid("format version 0 does not exist".to_owned()));
    }
    if from_version >= FORMAT_VERSION {
        return Ok(manifest);
    }
    let mut value = manifest;
    for version in from_version..FORMAT_VERSION {
        let step = STEPS.get(version as usize - 1).ok_or_else(|| {
            ProjectError::ManifestInvalid(format!("no migration from format version {version} is known"))
        })?;
        value = step(value)?;
    }
    if let Some(object) = value.as_object_mut() {
        object.insert("format_version".to_owned(), Value::from(FORMAT_VERSION));
    }
    Ok(value)
}

/// A manifest after version handling.
pub(crate) struct ParsedManifest {
    pub manifest: Manifest,
    /// `format_version` as found in the file (before any migration).
    pub found_version: u32,
    /// The file was written by a newer format version that this build can still read (`LoadWarning::NewerFormat`).
    pub newer: bool,
}

fn version_field(value: &Value, name: &str) -> Result<u32, ProjectError> {
    value
        .get(name)
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| ProjectError::ManifestInvalid(format!("\"{name}\" is missing or not a version number")))
}

/// The two-stage parse of section 7.3 for the bytes of `manifest.json`: JSON -> check `format` -> version handling
/// (`TooNew`, newer-but-readable, migration of older versions) -> typed `Manifest`.
pub(crate) fn parse_manifest(bytes: &[u8]) -> Result<ParsedManifest, ProjectError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|e| ProjectError::ManifestInvalid(e.to_string()))?;
    if value.get("format").and_then(Value::as_str) != Some("wtep") {
        return Err(ProjectError::ManifestInvalid("\"format\" is not \"wtep\"".to_owned()));
    }
    let found_version = version_field(&value, "format_version")?;
    let min_reader_version = version_field(&value, "min_reader_version")?;
    if min_reader_version > FORMAT_VERSION {
        return Err(ProjectError::TooNew { found: found_version.max(min_reader_version), supported: FORMAT_VERSION });
    }
    let newer = found_version > FORMAT_VERSION;
    let value = if found_version < FORMAT_VERSION { upgrade(value, found_version)? } else { value };
    let manifest: Manifest =
        serde_json::from_value(value).map_err(|e| ProjectError::ManifestInvalid(e.to_string()))?;
    Ok(ParsedManifest { manifest, found_version, newer })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn minimal(format_version: u32, min_reader_version: u32) -> Value {
        json!({
            "format": "wtep",
            "format_version": format_version,
            "min_reader_version": min_reader_version,
            "app_version": "0.1.0",
            "name": "demo",
            "saved_unix": 0,
            "next_id": 5,
            "source": {
                "format": "obj",
                "main": { "key": "a.obj", "original_name": "A.obj",
                          "file": { "entry": "source/main.obj", "size": 1, "crc32": 2 } },
                "sidecars": [],
                "original_path": null
            },
            "import": { "generate_fallback_uvs": false, "max_texture_dim": 8192, "importer_version": 1,
                        "uv_hash": "0123456789abcdef", "vertex_count": 3, "triangle_count": 1 },
            "sets": []
        })
    }

    fn parse(value: &Value) -> Result<ParsedManifest, ProjectError> {
        parse_manifest(&serde_json::to_vec(value).unwrap())
    }

    #[test]
    fn upgrade_is_the_identity_for_the_current_version() {
        let value = minimal(FORMAT_VERSION, 1);
        assert_eq!(upgrade(value.clone(), FORMAT_VERSION).unwrap(), value);
        // Newer manifests are not touched either.
        let newer = minimal(FORMAT_VERSION + 3, 1);
        assert_eq!(upgrade(newer.clone(), FORMAT_VERSION + 3).unwrap(), newer);
        assert!(matches!(upgrade(value, 0), Err(ProjectError::ManifestInvalid(_))));
        // One step per version below the current one.
        assert_eq!(STEPS.len() as u32, FORMAT_VERSION - 1);
    }

    #[test]
    fn current_version_parses() {
        let parsed = parse(&minimal(FORMAT_VERSION, 1)).unwrap();
        assert_eq!(parsed.found_version, FORMAT_VERSION);
        assert!(!parsed.newer);
        assert_eq!(parsed.manifest.name, "demo");
        assert_eq!(parsed.manifest.format_version, FORMAT_VERSION);
    }

    #[test]
    fn too_new_and_newer_but_readable() {
        match parse(&minimal(999, 999)) {
            Err(ProjectError::TooNew { found: 999, supported }) => assert_eq!(supported, FORMAT_VERSION),
            other => panic!("expected TooNew, got {:?}", other.map(|p| p.found_version)),
        }
        // A newer writer that declares this reader as sufficient: opened, flagged, unknown fields ignored.
        let mut value = minimal(FORMAT_VERSION + 1, FORMAT_VERSION);
        value["some_future_field"] = json!({ "x": 1 });
        let parsed = parse(&value).unwrap();
        assert!(parsed.newer);
        assert_eq!(parsed.found_version, FORMAT_VERSION + 1);
        assert_eq!(parsed.manifest.format_version, FORMAT_VERSION + 1);
    }

    #[test]
    fn invalid_manifests() {
        let invalid = |r: Result<ParsedManifest, ProjectError>| matches!(r, Err(ProjectError::ManifestInvalid(_)));
        assert!(invalid(parse_manifest(b"not json")));
        assert!(invalid(parse_manifest(b"[1, 2]")));
        let mut wrong_format = minimal(1, 1);
        wrong_format["format"] = json!("zip");
        assert!(invalid(parse(&wrong_format)));
        let mut no_version = minimal(1, 1);
        no_version.as_object_mut().unwrap().remove("format_version");
        assert!(invalid(parse(&no_version)));
        let mut bad_version = minimal(1, 1);
        bad_version["min_reader_version"] = json!("one");
        assert!(invalid(parse(&bad_version)));
        let mut zero = minimal(1, 1);
        zero["format_version"] = json!(0);
        assert!(invalid(parse(&zero)));
        let mut missing_field = minimal(1, 1);
        missing_field.as_object_mut().unwrap().remove("source");
        assert!(invalid(parse(&missing_field)));
    }
}
