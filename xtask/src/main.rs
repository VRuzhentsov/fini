use std::env;
use std::fs;
use std::path::Path;

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
        _ => Err(usage()),
    }
}

fn usage() -> String {
    "Usage: cargo run --manifest-path xtask/Cargo.toml -- release-version x.y.z".to_string()
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
    let raw = fs::read_to_string(path).map_err(|error| format!("read {}: {error}", path.display()))?;
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
