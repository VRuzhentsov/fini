use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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
        "release-notes" => {
            let from = args.next().ok_or_else(usage)?;
            let to = args.next().ok_or_else(usage)?;
            let output = args.next().ok_or_else(usage)?;
            if args.next().is_some() {
                return Err(usage());
            }
            generate_release_notes(&from, &to, Path::new(&output))
        }
        _ => Err(usage()),
    }
}

fn usage() -> String {
    "Usage: cargo run --manifest-path xtask/Cargo.toml -- <release-version x.y.z|play-store-screenshots|release-notes <from-tag> <to-tag> <output-file>>".to_string()
}

#[derive(Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ReleaseArea {
    Android,
    Cli,
    Core,
    Desktop,
    Distribution,
}

impl ReleaseArea {
    fn heading(&self) -> &'static str {
        match self {
            Self::Android => "Android",
            Self::Cli => "CLI",
            Self::Core => "Core",
            Self::Desktop => "Desktop",
            Self::Distribution => "Distribution",
        }
    }
}

#[derive(Debug)]
struct ReleaseEntry {
    area: ReleaseArea,
    kind: &'static str,
    text: String,
}

fn generate_release_notes(from: &str, to: &str, output: &Path) -> Result<(), String> {
    let commits = release_note_subjects(from, to)?;
    let release_kind = release_kind(from, to)?;
    let entries = commits
        .lines()
        .filter_map(parse_release_entry)
        .collect::<Vec<_>>();
    let notes = render_release_notes(release_kind, &entries);

    fs::write(output, notes).map_err(|error| format!("write {}: {error}", output.display()))?;
    println!(
        "wrote {} release-note entries to {}",
        entries.len(),
        output.display()
    );
    Ok(())
}

fn release_note_subjects(from: &str, to: &str) -> Result<String, String> {
    release_note_subjects_from_dir(from, to, None)
}

fn release_note_subjects_from_dir(
    from: &str,
    to: &str,
    current_dir: Option<&Path>,
) -> Result<String, String> {
    git_output_in(
        &[
            "log",
            "--first-parent",
            "--format=%s",
            &format!("{from}..{to}"),
        ],
        current_dir,
    )
}

fn git_output_in(args: &[&str], current_dir: Option<&Path>) -> Result<String, String> {
    let mut command = Command::new("git");
    command.args(args);
    if let Some(current_dir) = current_dir {
        command.current_dir(current_dir);
    }
    let output = command
        .output()
        .map_err(|error| format!("run git {}: {error}", args.join(" ")))?;
    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|error| format!("read git output: {error}"))
    } else {
        Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn release_kind(from: &str, to: &str) -> Result<&'static str, String> {
    let from = parse_tag_version(from)?;
    let to = parse_tag_version(to)?;
    if to.0 > from.0 || to.1 > from.1 {
        Ok("minor")
    } else {
        Ok("patch")
    }
}

fn parse_tag_version(tag: &str) -> Result<(u64, u64, u64), String> {
    let version = tag.strip_prefix('v').unwrap_or(tag);
    let version = version
        .split_once('-')
        .map_or(version, |(stable, _)| stable);
    let parts = version.split('.').collect::<Vec<_>>();
    if parts.len() != 3 {
        return Err(format!("release tag must be vMAJOR.MINOR.PATCH, got {tag}"));
    }
    let parse = |part: &str| {
        part.parse::<u64>()
            .map_err(|_| format!("invalid release tag {tag}"))
    };
    Ok((parse(parts[0])?, parse(parts[1])?, parse(parts[2])?))
}

fn parse_release_entry(subject: &str) -> Option<ReleaseEntry> {
    let (kind, scope, text) = match parse_conventional_subject(subject) {
        Some((kind, _scope, _text))
            if matches!(kind, "build" | "chore" | "ci" | "docs" | "release" | "test") =>
        {
            return None
        }
        Some((kind, scope, text)) => (kind, scope, text),
        None if subject.starts_with("Release ") || subject.starts_with("Merge ") => return None,
        None => (infer_subject_kind(subject), None, strip_pr_suffix(subject)),
    };

    let kind = match kind {
        "feat" | "new" => "New",
        "fix" | "bugfix" => "Bugfixes",
        "perf" | "refactor" | "improvement" => "Improvements",
        _ => "Improvements",
    };
    Some(ReleaseEntry {
        area: area_for_subject(scope, text),
        kind,
        text: text.to_string(),
    })
}

fn infer_subject_kind(subject: &str) -> &'static str {
    let subject = subject.to_ascii_lowercase();
    if subject.starts_with("fix ")
        || subject.starts_with("fixed ")
        || subject.starts_with("prevent ")
        || subject.starts_with("correct ")
        || subject.starts_with("repair ")
    {
        "bugfix"
    } else if subject.starts_with("add ")
        || subject.starts_with("enable ")
        || subject.starts_with("support ")
        || subject.starts_with("introduce ")
        || subject.starts_with("search ")
    {
        "new"
    } else {
        "improvement"
    }
}

fn parse_conventional_subject(subject: &str) -> Option<(&str, Option<&str>, &str)> {
    let (prefix, text) = subject.split_once(": ")?;
    let (kind, scope) = match prefix.split_once('(') {
        Some((kind, scoped)) => {
            let scoped = scoped.trim_end_matches('!');
            (kind, Some(scoped.strip_suffix(')')?))
        }
        None => (prefix.trim_end_matches('!'), None),
    };
    if !is_known_conventional_kind(kind) {
        return None;
    }
    let text = strip_pr_suffix(text);
    Some((kind, scope, text))
}

fn is_known_conventional_kind(kind: &str) -> bool {
    matches!(
        kind,
        "feat"
            | "fix"
            | "bugfix"
            | "perf"
            | "refactor"
            | "improvement"
            | "build"
            | "chore"
            | "ci"
            | "docs"
            | "release"
            | "test"
    )
}

fn strip_pr_suffix(text: &str) -> &str {
    text.strip_suffix(')')
        .and_then(|text| text.rsplit_once(" (#").map(|(text, _)| text))
        .unwrap_or(text)
}

fn area_for_subject(scope: Option<&str>, text: &str) -> ReleaseArea {
    let subject = format!("{} {}", scope.unwrap_or_default(), text).to_ascii_lowercase();
    let has_word = |word: &str| subject_words(&subject).any(|subject_word| subject_word == word);
    if has_word("android") {
        ReleaseArea::Android
    } else if has_word("cli") {
        ReleaseArea::Cli
    } else if has_word("release")
        || has_word("package")
        || has_word("appimage")
        || has_word("linux")
        || has_word("windows")
    {
        ReleaseArea::Distribution
    } else if has_word("desktop")
        || has_word("app")
        || has_word("settings")
        || has_word("ui")
        || has_word("updater")
    {
        ReleaseArea::Desktop
    } else {
        ReleaseArea::Core
    }
}

fn subject_words(subject: &str) -> impl Iterator<Item = &str> {
    subject
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
}

fn render_release_notes(release_kind: &str, entries: &[ReleaseEntry]) -> String {
    let mut grouped: BTreeMap<&ReleaseArea, BTreeMap<&str, Vec<&str>>> = BTreeMap::new();
    for entry in entries {
        let kind = if release_kind == "patch" && entry.kind == "New" {
            "Improvements"
        } else {
            entry.kind
        };
        grouped
            .entry(&entry.area)
            .or_default()
            .entry(kind)
            .or_default()
            .push(&entry.text);
    }

    let mut output = String::from("<!-- Generated from conventional commit subjects. Edit only by changing the source PR title/commit before tagging. -->\n\n");
    if entries.is_empty() {
        output.push_str("## Improvements\n\n- Maintenance release with no user-facing changes.\n");
        return output;
    }

    let kinds = if release_kind == "minor" {
        ["New", "Improvements", "Bugfixes"]
    } else {
        ["Bugfixes", "Improvements", "New"]
    };
    for (area, changes) in grouped {
        output.push_str(&format!("## {}\n\n", area.heading()));
        for kind in kinds {
            let Some(items) = changes.get(kind) else {
                continue;
            };
            output.push_str(&format!("### {kind}\n\n"));
            for item in items {
                output.push_str(&format!("- {item}\n"));
            }
            output.push('\n');
        }
    }
    output
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn run_git(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("git command should run");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn write_file(repo: &Path, path: &str, contents: &str) {
        let path = repo.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("test parent directory should be created");
        }
        fs::write(path, contents).expect("test file should be written");
    }

    fn commit(repo: &Path, path: &str, contents: &str, subject: &str) {
        write_file(repo, path, contents);
        run_git(repo, &["add", path]);
        run_git(repo, &["commit", "-m", subject]);
    }

    fn temp_repo() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let repo = env::temp_dir().join(format!("fini-xtask-release-notes-{unique}"));
        fs::create_dir_all(&repo).expect("test repo should be created");
        run_git(&repo, &["init", "-b", "main"]);
        run_git(&repo, &["config", "user.email", "test@example.com"]);
        run_git(&repo, &["config", "user.name", "Release Notes Test"]);
        repo
    }

    #[test]
    fn creates_minor_notes_from_conventional_and_pr_titles() {
        let entries = [
            parse_release_entry("feat(cli): add export command (#12)"),
            parse_release_entry("Fix AppImage Wayland runtime compatibility"),
            parse_release_entry("chore: release v0.2.0"),
        ];
        let entries = entries.into_iter().flatten().collect::<Vec<_>>();
        let notes = render_release_notes("minor", &entries);

        assert!(notes.contains("## CLI\n\n### New\n\n- add export command"));
        assert!(notes.contains(
            "## Distribution\n\n### Bugfixes\n\n- Fix AppImage Wayland runtime compatibility"
        ));
        assert!(!notes.contains("release v0.2.0"));
    }

    #[test]
    fn treats_new_work_in_patch_releases_as_an_improvement() {
        let entries = vec![parse_release_entry("feat(android): add offline queue").unwrap()];
        let notes = render_release_notes("patch", &entries);

        assert!(notes.contains("## Android\n\n### Improvements\n\n- add offline queue"));
        assert!(!notes.contains("### New"));
    }

    #[test]
    fn parses_scoped_breaking_conventional_subjects() {
        let entry = parse_release_entry("feat(cli)!: add export").unwrap();

        assert_eq!(entry.kind, "New");
        assert_eq!(entry.area, ReleaseArea::Cli);
        assert_eq!(entry.text, "add export");
    }

    #[test]
    fn falls_back_for_non_conventional_colon_titles() {
        let entry = parse_release_entry("Fix settings: prevent crash (#123)").unwrap();

        assert_eq!(entry.kind, "Bugfixes");
        assert_eq!(entry.area, ReleaseArea::Desktop);
        assert_eq!(entry.text, "Fix settings: prevent crash");
    }

    #[test]
    fn matches_release_note_areas_by_words() {
        assert_eq!(
            parse_release_entry("feat: add quick capture").unwrap().area,
            ReleaseArea::Core
        );
        assert_eq!(
            parse_release_entry("fix: apply defaults").unwrap().area,
            ReleaseArea::Core
        );
        assert_eq!(
            parse_release_entry("fix: click target").unwrap().area,
            ReleaseArea::Core
        );
        assert_eq!(
            parse_release_entry("fix(cli): click target").unwrap().area,
            ReleaseArea::Cli
        );
    }

    #[test]
    fn parses_prerelease_tags_as_semver() {
        assert_eq!(release_kind("v0.2.4", "v0.3.0-rc.1").unwrap(), "minor");
        assert_eq!(release_kind("v0.2.4", "v0.2.5").unwrap(), "patch");
    }

    #[test]
    fn release_note_subjects_follow_first_parent_history() {
        let repo = temp_repo();

        commit(&repo, "file.txt", "base\n", "chore: base");
        run_git(&repo, &["tag", "v0.1.0"]);
        run_git(&repo, &["checkout", "-b", "feature"]);
        commit(
            &repo,
            "feature.txt",
            "wip\n",
            "WIP branch implementation detail",
        );
        commit(&repo, "feature.txt", "ci\n", "ci: branch-only fixup");
        run_git(&repo, &["checkout", "main"]);
        commit(&repo, "main.txt", "main\n", "fix: first-parent fix");
        run_git(
            &repo,
            &[
                "merge",
                "--no-ff",
                "feature",
                "-m",
                "Merge pull request #123 from feature",
            ],
        );

        let subjects = release_note_subjects_from_dir("v0.1.0", "HEAD", Some(&repo)).unwrap();

        assert!(subjects.contains("Merge pull request #123 from feature"));
        assert!(subjects.contains("fix: first-parent fix"));
        assert!(!subjects.contains("WIP branch implementation detail"));
        assert!(!subjects.contains("ci: branch-only fixup"));

        fs::remove_dir_all(repo).expect("test repo should be removed");
    }
}
