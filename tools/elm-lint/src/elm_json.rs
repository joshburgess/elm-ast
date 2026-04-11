//! Parse `elm.json` to extract dependency information.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Deserialize;

/// Parsed dependency info from elm.json.
#[derive(Debug)]
pub struct ElmJsonInfo {
    /// Direct dependencies: package name -> version constraint string.
    pub direct_deps: HashMap<String, String>,
    /// Whether this is an application (vs a package).
    pub is_application: bool,
}

/// Errors that can occur when loading elm.json.
#[derive(Debug)]
pub enum ElmJsonError {
    Io(std::io::Error),
    Parse(serde_json::Error),
}

impl std::fmt::Display for ElmJsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ElmJsonError::Io(e) => write!(f, "could not read elm.json: {e}"),
            ElmJsonError::Parse(e) => write!(f, "could not parse elm.json: {e}"),
        }
    }
}

/// Load and parse elm.json from the given directory (or its parents).
pub fn load_elm_json(start_dir: &Path) -> Result<ElmJsonInfo, ElmJsonError> {
    let path = find_elm_json(start_dir).ok_or_else(|| {
        ElmJsonError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "elm.json not found",
        ))
    })?;
    let contents = std::fs::read_to_string(&path).map_err(ElmJsonError::Io)?;
    parse_elm_json(&contents)
}

/// Walk up from `start_dir` looking for elm.json.
fn find_elm_json(start_dir: &Path) -> Option<std::path::PathBuf> {
    let mut dir = start_dir.to_path_buf();
    loop {
        let candidate = dir.join("elm.json");
        if candidate.exists() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Parse elm.json content string into ElmJsonInfo.
pub fn parse_elm_json(contents: &str) -> Result<ElmJsonInfo, ElmJsonError> {
    // Try application format first, then package format.
    if let Ok(app) = serde_json::from_str::<ApplicationElmJson>(contents) {
        if app.type_field == "application" {
            return Ok(ElmJsonInfo {
                direct_deps: app.dependencies.direct,
                is_application: true,
            });
        }
    }
    if let Ok(pkg) = serde_json::from_str::<PackageElmJson>(contents) {
        if pkg.type_field == "package" {
            return Ok(ElmJsonInfo {
                direct_deps: pkg.dependencies,
                is_application: false,
            });
        }
    }
    // Fallback: try generic parse.
    Err(ElmJsonError::Parse(serde_json::from_str::<()>(contents).unwrap_err()))
}

// ── elm.json formats ─────────────────────────────────────────────────

/// Application elm.json format.
#[derive(Deserialize)]
struct ApplicationElmJson {
    #[serde(rename = "type")]
    type_field: String,
    dependencies: AppDependencies,
}

#[derive(Deserialize)]
struct AppDependencies {
    direct: HashMap<String, String>,
    // indirect deps exist but we don't need them.
}

/// Package elm.json format.
#[derive(Deserialize)]
struct PackageElmJson {
    #[serde(rename = "type")]
    type_field: String,
    dependencies: HashMap<String, String>,
}

// ── Package → module mapping ─────────────────────────────────────────

/// Known mapping of Elm package names to the modules they expose.
/// Covers the standard library and the most popular community packages.
pub fn known_package_modules() -> HashMap<&'static str, &'static [&'static str]> {
    let mut m = HashMap::new();

    // elm/* standard packages
    m.insert(
        "elm/core",
        [
            "Array", "Basics", "Bitwise", "Char", "Debug", "Dict", "List",
            "Maybe", "Order", "Platform", "Platform.Cmd", "Platform.Sub",
            "Process", "Result", "Set", "String", "Task", "Tuple",
        ]
        .as_slice(),
    );
    m.insert("elm/json", ["Json.Decode", "Json.Encode"].as_slice());
    m.insert(
        "elm/html",
        ["Html", "Html.Attributes", "Html.Events", "Html.Keyed", "Html.Lazy"].as_slice(),
    );
    m.insert("elm/http", ["Http"].as_slice());
    m.insert(
        "elm/browser",
        [
            "Browser", "Browser.Dom", "Browser.Events",
            "Browser.Navigation",
        ]
        .as_slice(),
    );
    m.insert("elm/url", ["Url", "Url.Builder", "Url.Parser", "Url.Parser.Query"].as_slice());
    m.insert("elm/time", ["Time"].as_slice());
    m.insert("elm/regex", ["Regex"].as_slice());
    m.insert(
        "elm/parser",
        ["Parser", "Parser.Advanced"].as_slice(),
    );
    m.insert("elm/random", ["Random"].as_slice());
    m.insert("elm/file", ["File", "File.Download", "File.Select"].as_slice());
    m.insert("elm/bytes", ["Bytes", "Bytes.Decode", "Bytes.Encode"].as_slice());
    m.insert(
        "elm/svg",
        ["Svg", "Svg.Attributes", "Svg.Events", "Svg.Keyed", "Svg.Lazy"].as_slice(),
    );
    m.insert(
        "elm/virtual-dom",
        ["VirtualDom"].as_slice(),
    );

    // Popular community packages
    m.insert(
        "elm-community/list-extra",
        ["List.Extra"].as_slice(),
    );
    m.insert("elm-community/maybe-extra", ["Maybe.Extra"].as_slice());
    m.insert("elm-community/string-extra", ["String.Extra"].as_slice());
    m.insert("elm-community/dict-extra", ["Dict.Extra"].as_slice());
    m.insert("elm-community/array-extra", ["Array.Extra"].as_slice());
    m.insert("elm-community/result-extra", ["Result.Extra"].as_slice());
    m.insert("elm-community/html-extra", ["Html.Extra", "Html.Attributes.Extra", "Html.Events.Extra"].as_slice());
    m.insert("elm-community/json-extra", ["Json.Decode.Extra"].as_slice());
    m.insert(
        "NoRedInk/elm-json-decode-pipeline",
        ["Json.Decode.Pipeline"].as_slice(),
    );
    m.insert(
        "krisajenern/remotedata",
        ["RemoteData"].as_slice(),
    );
    m.insert(
        "mdgriffith/elm-ui",
        ["Element", "Element.Background", "Element.Border", "Element.Events",
         "Element.Font", "Element.Input", "Element.Keyed", "Element.Lazy",
         "Element.Region"].as_slice(),
    );
    m.insert(
        "rtfeldman/elm-css",
        ["Css", "Css.Animations", "Css.Global", "Css.Media",
         "Css.Preprocess", "Css.Transitions", "Html.Styled",
         "Html.Styled.Attributes", "Html.Styled.Events",
         "Html.Styled.Keyed", "Html.Styled.Lazy",
         "Svg.Styled", "Svg.Styled.Attributes", "Svg.Styled.Events"].as_slice(),
    );
    m.insert("elm/project-metadata-utils", ["Elm.Docs", "Elm.Module", "Elm.Package", "Elm.Project", "Elm.Type", "Elm.Version", "Elm.Constraint", "Elm.License"].as_slice());
    m.insert("elm-explorations/test", ["Test", "Test.Runner", "Expect", "Fuzz"].as_slice());
    m.insert("elm-explorations/markdown", ["Markdown"].as_slice());
    m.insert("elm-explorations/linear-algebra", ["Math.Vector2", "Math.Vector3", "Math.Vector4", "Math.Matrix4"].as_slice());
    m.insert("elm-explorations/webgl", ["WebGL", "WebGL.Settings", "WebGL.Settings.Blend", "WebGL.Settings.DepthTest", "WebGL.Settings.StencilTest", "WebGL.Texture"].as_slice());
    m.insert("elm-explorations/benchmark", ["Benchmark", "Benchmark.Runner"].as_slice());

    m
}

/// Given a set of imported module names and the known package mapping, return
/// the set of package names that have at least one module imported.
pub fn packages_used_by_imports(
    imported_modules: &HashSet<String>,
    package_modules: &HashMap<&str, &[&str]>,
) -> HashSet<String> {
    let mut used = HashSet::new();
    for (pkg, modules) in package_modules {
        for module in *modules {
            if imported_modules.contains(*module) {
                used.insert(pkg.to_string());
                break;
            }
        }
    }
    used
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_application_elm_json() {
        let json = r#"{
            "type": "application",
            "source-directories": ["src"],
            "elm-version": "0.19.1",
            "dependencies": {
                "direct": {
                    "elm/core": "1.0.5",
                    "elm/html": "1.0.0",
                    "elm/json": "1.1.3"
                },
                "indirect": {
                    "elm/virtual-dom": "1.0.3"
                }
            },
            "test-dependencies": {
                "direct": {},
                "indirect": {}
            }
        }"#;

        let info = parse_elm_json(json).unwrap();
        assert!(info.is_application);
        assert_eq!(info.direct_deps.len(), 3);
        assert!(info.direct_deps.contains_key("elm/core"));
        assert!(info.direct_deps.contains_key("elm/html"));
        assert!(info.direct_deps.contains_key("elm/json"));
    }

    #[test]
    fn parse_package_elm_json() {
        let json = r#"{
            "type": "package",
            "name": "author/my-package",
            "summary": "A package",
            "license": "BSD-3-Clause",
            "version": "1.0.0",
            "exposed-modules": ["MyModule"],
            "elm-version": "0.19.0 <= v < 0.20.0",
            "dependencies": {
                "elm/core": "1.0.0 <= v < 2.0.0",
                "elm/json": "1.0.0 <= v < 2.0.0"
            },
            "test-dependencies": {}
        }"#;

        let info = parse_elm_json(json).unwrap();
        assert!(!info.is_application);
        assert_eq!(info.direct_deps.len(), 2);
        assert!(info.direct_deps.contains_key("elm/core"));
    }

    #[test]
    fn packages_used_by_imports_finds_matches() {
        let package_modules = known_package_modules();
        let mut imports = HashSet::new();
        imports.insert("Json.Decode".to_string());
        imports.insert("Html".to_string());

        let used = packages_used_by_imports(&imports, &package_modules);
        assert!(used.contains("elm/json"));
        assert!(used.contains("elm/html"));
        assert!(!used.contains("elm/http"));
    }

    #[test]
    fn packages_used_by_imports_no_match() {
        let package_modules = known_package_modules();
        let imports = HashSet::new();

        let used = packages_used_by_imports(&imports, &package_modules);
        assert!(used.is_empty());
    }
}
