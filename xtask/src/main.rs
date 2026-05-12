use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Value};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let command = args.next().ok_or_else(usage)?;

    match command.as_str() {
        "release-version" => {
            let version = args.next().ok_or_else(usage)?;
            if args.next().is_some() {
                return Err(usage());
            }
            set_release_version(&version)
        }
        "play-store-screenshots" => {
            if args.next().is_some() {
                return Err(usage());
            }
            prepare_play_store_screenshots()
        }
        _ => Err(usage()),
    }
}

fn usage() -> String {
    "Usage: cargo run --manifest-path xtask/Cargo.toml -- <release-version x.y.z|play-store-screenshots>".to_string()
}

struct ScreenshotSpec {
    device: &'static str,
    directory: &'static str,
    width: u32,
    height: u32,
    files: &'static [(&'static str, &'static str, &'static str)],
}

const SCREENSHOT_FILES: &[(&str, &str, &str)] = &[
    ("01-focus.png", "Focus", "One quest at a time"),
    (
        "02-history.png",
        "History",
        "Finish or abandon without pile-up",
    ),
    ("03-settings.png", "Settings", "Local-first and private"),
];

const SCREENSHOT_SPECS: &[ScreenshotSpec] = &[
    ScreenshotSpec {
        device: "phone",
        directory: "docs/play-store/screenshots/phone",
        width: 780,
        height: 1387,
        files: SCREENSHOT_FILES,
    },
    ScreenshotSpec {
        device: "tablet-7",
        directory: "docs/play-store/screenshots/tablet-7",
        width: 1200,
        height: 1920,
        files: SCREENSHOT_FILES,
    },
    ScreenshotSpec {
        device: "tablet-10",
        directory: "docs/play-store/screenshots/tablet-10",
        width: 1600,
        height: 2560,
        files: SCREENSHOT_FILES,
    },
];

fn prepare_play_store_screenshots() -> Result<(), String> {
    let mut screenshots = Vec::new();

    for spec in SCREENSHOT_SPECS {
        for (file_name, surface, caption) in spec.files {
            let path = PathBuf::from(spec.directory).join(file_name);
            validate_png_dimensions(&path, spec.width, spec.height)?;
            screenshots.push(serde_json::json!({
                "device": spec.device,
                "file": file_name,
                "path": path.to_string_lossy(),
                "width": spec.width,
                "height": spec.height,
                "surface": surface,
                "caption": caption,
                "theme": "canonical"
            }));
        }
    }

    let manifest_path = Path::new("docs/play-store/screenshots/manifest.json");
    let manifest = serde_json::json!({
        "market": "google-play",
        "generated_by": "cargo xtask play-store-screenshots",
        "listing": "docs/play-store/listing.md",
        "screenshots": screenshots
    });
    write_json(manifest_path, &manifest)?;
    println!("validated {} Play Store screenshots", screenshots.len());
    println!("wrote {}", manifest_path.display());
    Ok(())
}

fn validate_png_dimensions(
    path: &Path,
    expected_width: u32,
    expected_height: u32,
) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let (width, height) = png_dimensions(&bytes)
        .ok_or_else(|| format!("{} is not a PNG with a readable IHDR", path.display()))?;

    if width == expected_width && height == expected_height {
        Ok(())
    } else {
        Err(format!(
            "{} must be {}x{}, got {}x{}",
            path.display(),
            expected_width,
            expected_height,
            width,
            height
        ))
    }
}

fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 24 || &bytes[0..8] != PNG_SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }

    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((width, height))
}

fn set_release_version(version: &str) -> Result<(), String> {
    validate_semver(version)?;

    update_package_json("package.json", version)?;
    update_package_lock("package-lock.json", version)?;
    update_cargo_toml("src-tauri/Cargo.toml", version)?;
    update_cargo_lock("src-tauri/Cargo.lock", version)?;
    update_tauri_conf("src-tauri/tauri.conf.json", version)?;

    Ok(())
}

fn validate_semver(version: &str) -> Result<(), String> {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() == 3 && parts.iter().all(|part| is_decimal_number(part)) {
        return Ok(());
    }

    Err(format!("VERSION must match x.y.z, got {version}"))
}

fn is_decimal_number(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|character| character.is_ascii_digit())
}

fn read_json(path: impl AsRef<Path>) -> Result<serde_json::Value, String> {
    let path = path.as_ref();
    let raw =
        fs::read_to_string(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    serde_json::from_str(&raw).map_err(|error| format!("parse {}: {error}", path.display()))
}

fn write_json(path: impl AsRef<Path>, value: &serde_json::Value) -> Result<(), String> {
    let path = path.as_ref();
    let json = serde_json::to_string_pretty(value)
        .map_err(|error| format!("serialize {}: {error}", path.display()))?;
    fs::write(path, format!("{json}\n"))
        .map_err(|error| format!("write {}: {error}", path.display()))
}

fn update_package_json(path: &str, version: &str) -> Result<(), String> {
    let mut json = read_json(path)?;
    json["version"] = serde_json::Value::String(version.to_string());
    write_json(path, &json)
}

fn update_package_lock(path: &str, version: &str) -> Result<(), String> {
    let mut json = read_json(path)?;
    json["version"] = serde_json::Value::String(version.to_string());

    let root_package = json
        .get_mut("packages")
        .and_then(|packages| packages.get_mut(""))
        .ok_or_else(|| format!("{path} is missing packages[\"\"]"))?;
    root_package["version"] = serde_json::Value::String(version.to_string());

    write_json(path, &json)
}

fn update_tauri_conf(path: &str, version: &str) -> Result<(), String> {
    let mut json = read_json(path)?;
    json["version"] = serde_json::Value::String(version.to_string());
    write_json(path, &json)
}

fn update_cargo_toml(path: &str, version: &str) -> Result<(), String> {
    let mut doc = read_toml(path)?;
    let package = doc["package"]
        .as_table_mut()
        .ok_or_else(|| format!("{path} is missing [package]"))?;
    package["version"] = Item::Value(Value::from(version));
    write_toml(path, &doc)
}

fn update_cargo_lock(path: &str, version: &str) -> Result<(), String> {
    let mut doc = read_toml(path)?;
    let packages = doc["package"]
        .as_array_of_tables_mut()
        .ok_or_else(|| format!("{path} is missing [[package]] entries"))?;

    let mut updated = false;

    for package in packages.iter_mut() {
        let is_fini = package
            .get("name")
            .and_then(|name| name.as_str())
            .is_some_and(|name| name == "fini");

        if is_fini {
            package["version"] = Item::Value(Value::from(version));
            updated = true;
            break;
        }
    }

    if updated {
        write_toml(path, &doc)
    } else {
        Err(format!("{path} is missing the fini package entry"))
    }
}

fn read_toml(path: &str) -> Result<DocumentMut, String> {
    let raw = fs::read_to_string(path).map_err(|error| format!("read {path}: {error}"))?;
    raw.parse::<DocumentMut>()
        .map_err(|error| format!("parse {path}: {error}"))
}

fn write_toml(path: &str, doc: &DocumentMut) -> Result<(), String> {
    fs::write(path, doc.to_string()).map_err(|error| format!("write {path}: {error}"))
}
