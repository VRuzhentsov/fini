use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT};
use semver::Version;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::{self, Cursor};
use std::path::{Path, PathBuf};
use std::process::Command;

const DEFAULT_REPO: &str = "VRuzhentsov/fini";
const GITHUB_API: &str = "https://api.github.com";

#[derive(Debug, Clone)]
pub struct UpdateOptions {
    pub dry_run: bool,
    pub repo: Option<String>,
    pub install_bin: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveKind {
    TarGz,
    Zip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliAssetTarget {
    pub os: String,
    pub arch: String,
    pub executable_name: String,
    pub archive_kind: ArchiveKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePlan {
    pub repo: String,
    pub current_version: String,
    pub target_tag: String,
    pub target_version: String,
    pub asset_name: String,
    pub asset_url: String,
    pub asset_digest: Option<String>,
    pub install_dir: PathBuf,
    pub install_bin: PathBuf,
    pub target_binary: PathBuf,
    pub already_current: bool,
    archive_kind: ArchiveKind,
    executable_name: String,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

pub fn run_update(options: UpdateOptions) -> Result<Value, String> {
    let repo = options
        .repo
        .or_else(|| std::env::var("FINI_UPDATE_REPO").ok())
        .unwrap_or_else(|| DEFAULT_REPO.to_string());
    validate_repo(&repo)?;

    let target = current_cli_asset_target()?;
    let install_bin = options
        .install_bin
        .or_else(|| std::env::var_os("FINI_UPDATE_BIN_PATH").map(PathBuf::from))
        .unwrap_or_else(|| default_install_bin(&target));
    let releases = list_releases(&repo)?;
    let plan = select_update_plan(
        repo,
        env!("CARGO_PKG_VERSION"),
        &releases,
        target,
        install_bin,
    )?;

    if options.dry_run || plan.already_current {
        return Ok(plan_json(&plan, options.dry_run, false));
    }

    apply_update(&plan)?;
    Ok(plan_json(&plan, false, true))
}

fn list_releases(repo: &str) -> Result<Vec<GitHubRelease>, String> {
    let url = format!("{GITHUB_API}/repos/{repo}/releases");
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|err| format!("failed to create update runtime: {err}"))?;
    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .default_headers(github_headers()?)
            .build()
            .map_err(|err| format!("failed to create GitHub client: {err}"))?;
        let response = client
            .get(url)
            .send()
            .await
            .map_err(|err| format!("failed to query GitHub releases: {err}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "GitHub releases query failed with HTTP {}",
                response.status()
            ));
        }
        let text = response
            .text()
            .await
            .map_err(|err| format!("failed to read GitHub release response: {err}"))?;
        let releases: Vec<GitHubRelease> = serde_json::from_str(&text)
            .map_err(|err| format!("failed to parse GitHub release response: {err}"))?;
        Ok(releases)
    })
}

fn apply_update(plan: &UpdatePlan) -> Result<(), String> {
    let staging_dir = plan.install_dir.join(".download");
    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir)
            .map_err(|err| format!("failed to clear update staging dir: {err}"))?;
    }
    fs::create_dir_all(&staging_dir)
        .map_err(|err| format!("failed to create update staging dir: {err}"))?;
    fs::create_dir_all(&plan.install_dir)
        .map_err(|err| format!("failed to create install dir: {err}"))?;

    let archive_path = staging_dir.join(&plan.asset_name);
    download_asset(&plan.asset_url, &archive_path)?;
    verify_digest(&archive_path, plan.asset_digest.as_deref())?;
    extract_binary(
        &archive_path,
        &plan.target_binary,
        &plan.archive_kind,
        &plan.executable_name,
    )?;
    verify_candidate_binary(&plan.target_binary, &plan.target_version)?;
    activate_binary(&plan.target_binary, &plan.install_bin)?;
    fs::remove_dir_all(&staging_dir)
        .map_err(|err| format!("failed to remove update staging dir: {err}"))?;
    Ok(())
}

fn download_asset(url: &str, archive_path: &Path) -> Result<(), String> {
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|err| format!("failed to create download runtime: {err}"))?;
    let bytes = runtime.block_on(async {
        let client = reqwest::Client::builder()
            .default_headers(github_headers()?)
            .build()
            .map_err(|err| format!("failed to create download client: {err}"))?;
        let response = client
            .get(url)
            .send()
            .await
            .map_err(|err| format!("failed to download update asset: {err}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "release asset download failed with HTTP {}",
                response.status()
            ));
        }
        response
            .bytes()
            .await
            .map_err(|err| format!("failed to read update asset: {err}"))
    })?;
    fs::write(archive_path, bytes)
        .map_err(|err| format!("failed to write update archive: {err}"))?;
    Ok(())
}

fn github_headers() -> Result<HeaderMap, String> {
    github_headers_from_token(std::env::var("GH_TOKEN").ok())
}

fn github_headers_from_token(token: Option<String>) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static("fini-cli-updater"));
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    if let Some(token) = token {
        let token = token.trim();
        if !token.is_empty() {
            let value = HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|err| format!("invalid GH_TOKEN for GitHub API: {err}"))?;
            headers.insert(AUTHORIZATION, value);
        }
    }
    Ok(headers)
}

fn verify_digest(archive_path: &Path, expected: Option<&str>) -> Result<(), String> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let expected = expected
        .strip_prefix("sha256:")
        .unwrap_or(expected)
        .to_ascii_lowercase();
    let bytes = fs::read(archive_path)
        .map_err(|err| format!("failed to read update archive for digest check: {err}"))?;
    let actual = format!("{:x}", Sha256::digest(&bytes));
    if actual != expected {
        return Err(format!(
            "update archive digest mismatch: expected {expected}, got {actual}"
        ));
    }
    Ok(())
}

fn extract_binary(
    archive_path: &Path,
    target_binary: &Path,
    archive_kind: &ArchiveKind,
    executable_name: &str,
) -> Result<(), String> {
    let parent = target_binary
        .parent()
        .ok_or_else(|| "target binary path has no parent directory".to_string())?;
    fs::create_dir_all(parent).map_err(|err| format!("failed to create install dir: {err}"))?;

    match archive_kind {
        ArchiveKind::TarGz => extract_tar_gz_binary(archive_path, target_binary, executable_name)?,
        ArchiveKind::Zip => extract_zip_binary(archive_path, target_binary, executable_name)?,
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(target_binary)
            .map_err(|err| format!("failed to inspect extracted binary: {err}"))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(target_binary, permissions)
            .map_err(|err| format!("failed to mark extracted binary executable: {err}"))?;
    }

    Ok(())
}

fn extract_tar_gz_binary(
    archive_path: &Path,
    target_binary: &Path,
    executable_name: &str,
) -> Result<(), String> {
    let file = File::open(archive_path).map_err(|err| format!("failed to open archive: {err}"))?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    for entry in archive
        .entries()
        .map_err(|err| format!("failed to read tar archive: {err}"))?
    {
        let mut entry = entry.map_err(|err| format!("failed to read tar entry: {err}"))?;
        let path = entry
            .path()
            .map_err(|err| format!("failed to inspect tar entry path: {err}"))?;
        if path.file_name().and_then(|name| name.to_str()) == Some(executable_name) {
            let mut output = File::create(target_binary)
                .map_err(|err| format!("failed to create target binary: {err}"))?;
            io::copy(&mut entry, &mut output)
                .map_err(|err| format!("failed to extract CLI binary: {err}"))?;
            return Ok(());
        }
    }
    Err(format!("archive does not contain {executable_name}"))
}

fn extract_zip_binary(
    archive_path: &Path,
    target_binary: &Path,
    executable_name: &str,
) -> Result<(), String> {
    let bytes = fs::read(archive_path).map_err(|err| format!("failed to open archive: {err}"))?;
    let reader = Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(reader).map_err(|err| format!("failed to read zip archive: {err}"))?;
    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|err| format!("failed to read zip entry: {err}"))?;
        let name = Path::new(file.name())
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        if name == executable_name {
            let mut output = File::create(target_binary)
                .map_err(|err| format!("failed to create target binary: {err}"))?;
            io::copy(&mut file, &mut output)
                .map_err(|err| format!("failed to extract CLI binary: {err}"))?;
            return Ok(());
        }
    }
    Err(format!("archive does not contain {executable_name}"))
}

fn verify_candidate_binary(binary: &Path, target_version: &str) -> Result<(), String> {
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .map_err(|err| format!("failed to run extracted CLI binary: {err}"))?;
    if !output.status.success() {
        return Err("extracted CLI binary failed minimal version check".to_string());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.contains(target_version) {
        return Err(format!(
            "extracted CLI version check failed: expected {target_version}, got {}",
            stdout.trim()
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn activate_binary(target_binary: &Path, install_bin: &Path) -> Result<(), String> {
    use std::os::unix::fs::symlink;
    let parent = install_bin
        .parent()
        .ok_or_else(|| "install path has no parent directory".to_string())?;
    fs::create_dir_all(parent).map_err(|err| format!("failed to create bin dir: {err}"))?;
    let tmp_link = install_bin.with_extension("fini-update-tmp");
    if tmp_link.exists() {
        fs::remove_file(&tmp_link)
            .map_err(|err| format!("failed to remove stale update link: {err}"))?;
    }
    symlink(target_binary, &tmp_link)
        .map_err(|err| format!("failed to create update symlink: {err}"))?;
    fs::rename(&tmp_link, install_bin)
        .map_err(|err| format!("failed to activate updated CLI binary: {err}"))?;
    Ok(())
}

#[cfg(not(unix))]
fn activate_binary(target_binary: &Path, install_bin: &Path) -> Result<(), String> {
    let parent = install_bin
        .parent()
        .ok_or_else(|| "install path has no parent directory".to_string())?;
    fs::create_dir_all(parent).map_err(|err| format!("failed to create bin dir: {err}"))?;
    let tmp_bin = install_bin.with_extension("fini-update-tmp");
    if tmp_bin.exists() {
        fs::remove_file(&tmp_bin)
            .map_err(|err| format!("failed to remove stale staged CLI binary: {err}"))?;
    }
    if let Err(err) = fs::copy(target_binary, &tmp_bin) {
        let _ = fs::remove_file(&tmp_bin);
        return Err(format!("failed to stage updated CLI binary: {err}"));
    }
    replace_file_non_unix(&tmp_bin, install_bin)?;
    Ok(())
}

#[cfg(any(not(unix), test))]
fn replace_file_non_unix(staged: &Path, destination: &Path) -> Result<(), String> {
    if destination.exists() {
        if should_defer_non_unix_replacement(destination)? {
            return schedule_deferred_non_unix_replacement(staged, destination);
        }
        if let Err(err) = fs::remove_file(destination) {
            let _ = fs::remove_file(staged);
            return Err(format!("failed to replace existing CLI binary: {err}"));
        }
    }
    if let Err(err) = fs::rename(staged, destination) {
        let _ = fs::remove_file(staged);
        return Err(format!("failed to activate updated CLI binary: {err}"));
    }
    Ok(())
}

#[cfg(any(not(unix), test))]
fn should_defer_non_unix_replacement(destination: &Path) -> Result<bool, String> {
    let current_exe = std::env::current_exe()
        .map_err(|err| format!("failed to resolve current CLI binary: {err}"))?;
    replacement_targets_running_binary(destination, &current_exe)
}

#[cfg(any(not(unix), test))]
fn replacement_targets_running_binary(
    destination: &Path,
    current_exe: &Path,
) -> Result<bool, String> {
    let destination = destination
        .canonicalize()
        .map_err(|err| format!("failed to resolve existing CLI binary: {err}"))?;
    let current_exe = current_exe
        .canonicalize()
        .map_err(|err| format!("failed to resolve current CLI binary: {err}"))?;
    Ok(destination == current_exe)
}

#[cfg(windows)]
fn schedule_deferred_non_unix_replacement(staged: &Path, destination: &Path) -> Result<(), String> {
    let script = destination.with_extension("fini-update.cmd");
    let content = windows_replacement_script(staged, destination);
    fs::write(&script, content)
        .map_err(|err| format!("failed to write deferred CLI replacement helper: {err}"))?;
    Command::new("cmd")
        .args(["/C", "start", "", "/min"])
        .arg(&script)
        .spawn()
        .map_err(|err| format!("failed to start deferred CLI replacement helper: {err}"))?;
    Ok(())
}

#[cfg(all(not(unix), not(windows)))]
fn schedule_deferred_non_unix_replacement(
    staged: &Path,
    _destination: &Path,
) -> Result<(), String> {
    let _ = fs::remove_file(staged);
    Err("cannot replace the running CLI binary on this platform".to_string())
}

#[cfg(all(test, not(windows)))]
fn schedule_deferred_non_unix_replacement(
    _staged: &Path,
    _destination: &Path,
) -> Result<(), String> {
    Ok(())
}

#[cfg(any(windows, test))]
fn windows_replacement_script(staged: &Path, destination: &Path) -> String {
    let staged = batch_literal(staged);
    let destination = batch_literal(destination);
    format!(
        "@echo off\r\n\
         setlocal\r\n\
         set \"STAGED={staged}\"\r\n\
         set \"DESTINATION={destination}\"\r\n\
         for /l %%i in (1,1,30) do (\r\n\
         \u{20} move /Y \"%STAGED%\" \"%DESTINATION%\" >nul 2>nul\r\n\
         \u{20} if not exist \"%STAGED%\" (\r\n\
         \u{20}\u{20} del \"%~f0\" >nul 2>nul\r\n\
         \u{20}\u{20} exit /b 0\r\n\
         \u{20} )\r\n\
         \u{20} timeout /t 1 /nobreak >nul\r\n\
         )\r\n\
         exit /b 1\r\n"
    )
}

#[cfg(any(windows, test))]
fn batch_literal(path: &Path) -> String {
    path.to_string_lossy().replace('%', "%%")
}

fn build_update_plan(
    repo: String,
    current_version: String,
    target_tag: String,
    asset_name: String,
    asset_url: String,
    asset_digest: Option<String>,
    install_bin: PathBuf,
    target: CliAssetTarget,
) -> Result<UpdatePlan, String> {
    let target_version = target_tag
        .strip_prefix('v')
        .unwrap_or(&target_tag)
        .to_string();
    let version_order = compare_update_versions(&current_version, &target_version)?;
    if version_order == Ordering::Less {
        return Err(format!(
            "refusing to downgrade CLI from {current_version} to {target_version}"
        ));
    }
    let install_root = default_install_root()?;
    let install_dir = install_root.join(&target_tag);
    let target_binary = install_dir.join(&target.executable_name);
    Ok(UpdatePlan {
        repo,
        already_current: version_order == Ordering::Equal,
        current_version,
        target_tag,
        target_version,
        asset_name,
        asset_url,
        asset_digest,
        install_dir,
        install_bin,
        target_binary,
        archive_kind: target.archive_kind,
        executable_name: target.executable_name,
    })
}

fn compare_update_versions(
    current_version: &str,
    target_version: &str,
) -> Result<Ordering, String> {
    let current = Version::parse(current_version)
        .map_err(|err| format!("failed to parse current CLI version {current_version}: {err}"))?;
    let target = Version::parse(target_version)
        .map_err(|err| format!("failed to parse target CLI version {target_version}: {err}"))?;
    Ok(target.cmp(&current))
}

fn select_update_plan(
    repo: String,
    current_version: &str,
    releases: &[GitHubRelease],
    target: CliAssetTarget,
    install_bin: PathBuf,
) -> Result<UpdatePlan, String> {
    let release = releases
        .iter()
        .find(|release| !release.draft && !release.prerelease)
        .ok_or_else(|| "no stable GitHub release found".to_string())?;
    let asset_name = cli_asset_name(&release.tag_name, &target);
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == asset_name)
        .ok_or_else(|| format!("release {} does not include {asset_name}", release.tag_name))?;

    build_update_plan(
        repo,
        current_version.to_string(),
        release.tag_name.clone(),
        asset.name.clone(),
        asset.browser_download_url.clone(),
        asset.digest.clone(),
        install_bin,
        target,
    )
}

pub fn cli_asset_name(tag: &str, target: &CliAssetTarget) -> String {
    let extension = match target.archive_kind {
        ArchiveKind::TarGz => "tar.gz",
        ArchiveKind::Zip => "zip",
    };
    format!("fini-{tag}-{}-{}-cli.{extension}", target.os, target.arch)
}

pub fn current_cli_asset_target() -> Result<CliAssetTarget, String> {
    cli_asset_target(std::env::consts::OS, std::env::consts::ARCH)
}

pub fn cli_asset_target(os: &str, arch: &str) -> Result<CliAssetTarget, String> {
    let arch = match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => return Err(format!("unsupported CLI update architecture: {other}")),
    };
    match os {
        "linux" => Ok(CliAssetTarget {
            os: "linux".to_string(),
            arch: arch.to_string(),
            executable_name: "fini".to_string(),
            archive_kind: ArchiveKind::TarGz,
        }),
        "windows" => Ok(CliAssetTarget {
            os: "windows".to_string(),
            arch: arch.to_string(),
            executable_name: "fini.exe".to_string(),
            archive_kind: ArchiveKind::Zip,
        }),
        other => Err(format!("unsupported CLI update OS: {other}")),
    }
}

fn validate_repo(repo: &str) -> Result<(), String> {
    let parts: Vec<_> = repo.split('/').collect();
    if parts.len() != 2 || parts.iter().any(|part| part.is_empty()) {
        return Err("update repo must use owner/name format".to_string());
    }
    Ok(())
}

fn default_install_root() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|home| home.join(".local/lib/fini"))
        .ok_or_else(|| "could not determine home directory for CLI install".to_string())
}

fn default_install_bin(target: &CliAssetTarget) -> PathBuf {
    dirs::home_dir()
        .map(|home| home.join(".local/bin").join(&target.executable_name))
        .unwrap_or_else(|| PathBuf::from(&target.executable_name))
}

fn plan_json(plan: &UpdatePlan, dry_run: bool, updated: bool) -> Value {
    json!({
        "current_version": plan.current_version,
        "target_version": plan.target_version,
        "target_tag": plan.target_tag,
        "repo": plan.repo,
        "asset": plan.asset_name,
        "asset_digest": plan.asset_digest,
        "install_dir": plan.install_dir,
        "install_bin": plan.install_bin,
        "dry_run": dry_run,
        "already_current": plan.already_current,
        "updated": updated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linux_x64_target() -> CliAssetTarget {
        cli_asset_target("linux", "x86_64").expect("linux x64 target")
    }

    #[test]
    fn cli_asset_name_uses_release_archive_contract() {
        assert_eq!(
            cli_asset_name("v0.1.33", &linux_x64_target()),
            "fini-v0.1.33-linux-x64-cli.tar.gz"
        );
        let windows = cli_asset_target("windows", "aarch64").expect("windows arm64 target");
        assert_eq!(
            cli_asset_name("v0.1.33", &windows),
            "fini-v0.1.33-windows-arm64-cli.zip"
        );
    }

    #[test]
    fn default_install_bin_uses_target_executable_name() {
        let windows = cli_asset_target("windows", "x86_64").expect("windows x64 target");
        assert_eq!(
            default_install_bin(&windows),
            dirs::home_dir().expect("home").join(".local/bin/fini.exe")
        );

        assert_eq!(
            default_install_bin(&linux_x64_target()),
            dirs::home_dir().expect("home").join(".local/bin/fini")
        );
    }

    #[test]
    fn github_headers_include_authorization_when_token_is_present() {
        let headers = github_headers_from_token(Some("  test-token  ".to_string()))
            .expect("headers with token");

        assert_eq!(
            headers.get(AUTHORIZATION).expect("authorization header"),
            "Bearer test-token"
        );
    }

    #[test]
    fn update_plan_marks_matching_version_current() {
        let plan = build_update_plan(
            "example/fini".to_string(),
            "0.1.33".to_string(),
            "v0.1.33".to_string(),
            "fini-v0.1.33-linux-x64-cli.tar.gz".to_string(),
            "https://example.test/fini.tar.gz".to_string(),
            Some("sha256:abc".to_string()),
            PathBuf::from("/var/tmp/fini-test-bin/fini"),
            linux_x64_target(),
        )
        .expect("plan");

        assert!(plan.already_current);
        assert_eq!(plan.target_version, "0.1.33");
        assert_eq!(
            plan.target_binary,
            dirs::home_dir()
                .expect("home")
                .join(".local/lib/fini/v0.1.33/fini")
        );
    }

    #[test]
    fn update_plan_rejects_target_older_than_current_version() {
        let result = build_update_plan(
            "example/fini".to_string(),
            "0.1.34-rc.1".to_string(),
            "v0.1.33".to_string(),
            "fini-v0.1.33-linux-x64-cli.tar.gz".to_string(),
            "https://example.test/fini.tar.gz".to_string(),
            Some("sha256:abc".to_string()),
            PathBuf::from("/var/tmp/fini-test-bin/fini"),
            linux_x64_target(),
        );

        assert_eq!(
            result.expect_err("older target rejected"),
            "refusing to downgrade CLI from 0.1.34-rc.1 to 0.1.33"
        );
    }

    #[test]
    fn select_update_plan_chooses_stable_matching_asset_and_no_change() {
        let releases = vec![
            GitHubRelease {
                tag_name: "v0.1.34".to_string(),
                draft: false,
                prerelease: true,
                assets: vec![GitHubAsset {
                    name: "fini-v0.1.34-linux-x64-cli.tar.gz".to_string(),
                    browser_download_url: "https://example.test/pre.tar.gz".to_string(),
                    digest: None,
                }],
            },
            GitHubRelease {
                tag_name: "v0.1.33".to_string(),
                draft: false,
                prerelease: false,
                assets: vec![GitHubAsset {
                    name: "fini-v0.1.33-linux-x64-cli.tar.gz".to_string(),
                    browser_download_url: "https://example.test/stable.tar.gz".to_string(),
                    digest: Some("sha256:abc".to_string()),
                }],
            },
        ];

        let plan = select_update_plan(
            "example/fini".to_string(),
            "0.1.33",
            &releases,
            linux_x64_target(),
            PathBuf::from("/var/tmp/fini-test-bin/fini"),
        )
        .expect("selected update plan");

        assert!(plan.already_current);
        assert_eq!(plan.target_tag, "v0.1.33");
        assert_eq!(plan.asset_name, "fini-v0.1.33-linux-x64-cli.tar.gz");
        assert_eq!(
            plan.asset_url,
            "https://example.test/stable.tar.gz".to_string()
        );
    }

    #[test]
    fn digest_verification_rejects_mismatch() {
        let path = PathBuf::from("/var/tmp").join(format!(
            "fini-digest-test-{}-{}",
            std::process::id(),
            "mismatch"
        ));
        fs::write(&path, b"archive").expect("write test archive");
        let result = verify_digest(&path, Some("sha256:0000"));
        let _ = fs::remove_file(&path);

        assert!(result.is_err());
        assert!(result
            .expect_err("digest mismatch")
            .contains("digest mismatch"));
    }

    #[test]
    fn non_unix_replacement_overwrites_existing_destination() {
        let root = PathBuf::from("/var/tmp").join(format!(
            "fini-replace-test-{}-{}",
            std::process::id(),
            "overwrite"
        ));
        fs::create_dir_all(&root).expect("create temp dir");
        let staged = root.join("fini-update-tmp");
        let destination = root.join("fini.exe");
        fs::write(&staged, b"new").expect("write staged");
        fs::write(&destination, b"old").expect("write destination");

        replace_file_non_unix(&staged, &destination).expect("replace destination");

        assert!(!staged.exists());
        assert_eq!(fs::read(&destination).expect("read destination"), b"new");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn non_unix_replacement_cleans_staged_file_when_destination_cannot_be_removed() {
        let root = PathBuf::from("/var/tmp").join(format!(
            "fini-replace-test-{}-{}",
            std::process::id(),
            "cleanup"
        ));
        fs::create_dir_all(&root).expect("create temp dir");
        let staged = root.join("fini-update-tmp");
        let destination = root.join("fini.exe");
        fs::write(&staged, b"new").expect("write staged");
        fs::create_dir(&destination).expect("create blocking destination");

        let result = replace_file_non_unix(&staged, &destination);

        assert!(result.is_err());
        assert!(!staged.exists());
        assert!(destination.is_dir());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn non_unix_replacement_detects_running_destination() {
        let root = PathBuf::from("/var/tmp").join(format!(
            "fini-replace-test-{}-{}",
            std::process::id(),
            "running"
        ));
        fs::create_dir_all(&root).expect("create temp dir");
        fs::create_dir_all(root.join("nested")).expect("create nested temp dir");
        let current = root.join("fini.exe");
        let alias = root.join("nested").join("..").join("fini.exe");
        let other = root.join("other.exe");
        fs::write(&current, b"current").expect("write current");
        fs::write(&other, b"other").expect("write other");

        assert!(
            replacement_targets_running_binary(&alias, &current).expect("compare running target")
        );
        assert!(
            !replacement_targets_running_binary(&other, &current).expect("compare other target")
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn windows_replacement_script_escapes_percent_paths() {
        let staged = PathBuf::from(r"C:\Users\example\AppData\Local\fini%20folder\fini.tmp");
        let destination = PathBuf::from(r"C:\Users\example\.local\bin\fini.exe");

        let script = windows_replacement_script(&staged, &destination);

        assert!(script.contains(r"fini%%20folder"));
        assert!(script.contains("move /Y"));
        assert!(script.contains(r"%STAGED%"));
        assert!(script.contains(r"%DESTINATION%"));
    }
}
