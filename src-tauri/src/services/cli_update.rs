use flate2::read::GzDecoder;
use serde_json::{json, Value};
use std::ffi::OsStr;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

const CLI_UPDATE_IDENTIFIER: &str = "cli";
const CLI_UPDATE_VERIFYING_KEY: [u8; 32] = *include_bytes!("../../keys/fini-cli-zipsign.pub");

#[derive(Debug, Clone)]
pub struct UpdateOptions {
    pub dry_run: bool,
    pub endpoint: Option<String>,
    pub pubkey: Option<String>,
    pub target: Option<String>,
    pub executable_path: Option<PathBuf>,
}

fn cli_update_repository() -> Result<(&'static str, &'static str), String> {
    let repository = option_env!("FINI_CLI_UPDATE_REPOSITORY")
        .ok_or_else(|| "CLI update repository was not configured at build time".to_string())?;
    repository
        .split_once('/')
        .ok_or_else(|| "CLI update repository must use the owner/repository form".to_string())
}

pub fn run_update(options: UpdateOptions) -> Result<Value, String> {
    if options.endpoint.is_some() || options.pubkey.is_some() {
        return Err("custom Tauri updater endpoints and public keys are not supported by the standalone CLI updater".to_string());
    }

    let target = options
        .target
        .unwrap_or_else(|| self_update::get_target().to_string());
    let executable_path = options
        .executable_path
        .map(Ok)
        .unwrap_or_else(std::env::current_exe)
        .map_err(|err| format!("failed to resolve current CLI executable: {err}"))?;

    if options.dry_run {
        let (owner, repository) = cli_update_repository()?;
        let releases = self_update::backends::github::ReleaseList::configure()
            .repo_owner(owner)
            .repo_name(repository)
            .build()
            .map_err(|err| format!("failed to configure standalone CLI release check: {err}"))?
            .fetch()
            .map_err(|err| format!("failed to check standalone CLI releases: {err}"))?;
        let Some(release) = compatible_cli_release(&releases, &target) else {
            return Ok(json!({
                "current_version": env!("CARGO_PKG_VERSION"),
                "target_version": env!("CARGO_PKG_VERSION"),
                "target": target,
                "executable_path": executable_path,
                "dry_run": true,
                "already_current": true,
                "updated": false,
            }));
        };
        let target_version = release.version.trim_start_matches('v');
        return Ok(json!({
            "current_version": env!("CARGO_PKG_VERSION"),
            "target_version": target_version,
            "target": target,
            "executable_path": executable_path,
            "dry_run": true,
            "already_current": target_version == env!("CARGO_PKG_VERSION"),
            "updated": false,
        }));
    }

    let Some(release) = latest_cli_release(&target)? else {
        return Ok(json!({
            "current_version": env!("CARGO_PKG_VERSION"),
            "target_version": env!("CARGO_PKG_VERSION"),
            "target": target,
            "executable_path": executable_path,
            "dry_run": false,
            "already_current": true,
            "updated": false,
        }));
    };
    let target_version = release.version.trim_start_matches('v');
    if target_version == env!("CARGO_PKG_VERSION") {
        return Ok(json!({
            "current_version": env!("CARGO_PKG_VERSION"),
            "target_version": target_version,
            "target": target,
            "executable_path": executable_path,
            "dry_run": false,
            "already_current": true,
            "updated": false,
        }));
    }

    let asset = cli_asset_for_release(&release, &target)
        .ok_or_else(|| format!("no standalone CLI release asset found for {target}"))?;
    let payload = download_cli_asset(&asset.download_url)?;
    verify_cli_payload_signature(&payload, &asset.name, &target)?;
    install_verified_cli_payload(&payload, &target, target_version, &executable_path)?;

    Ok(json!({
        "current_version": env!("CARGO_PKG_VERSION"),
        "target_version": target_version,
        "target": target,
        "executable_path": executable_path,
        "dry_run": false,
        "already_current": false,
        "updated": true,
    }))
}

fn latest_cli_release(target: &str) -> Result<Option<self_update::update::Release>, String> {
    let (owner, repository) = cli_update_repository()?;
    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner(owner)
        .repo_name(repository)
        .build()
        .map_err(|err| format!("failed to configure standalone CLI release check: {err}"))?
        .fetch()
        .map_err(|err| format!("failed to check standalone CLI releases: {err}"))?;

    if releases.is_empty() {
        return Err(format!(
            "no standalone CLI release asset found for {target}"
        ));
    }

    Ok(compatible_cli_release(&releases, target))
}

fn compatible_cli_release(
    releases: &[self_update::update::Release],
    target: &str,
) -> Option<self_update::update::Release> {
    let newer_releases = releases
        .iter()
        .filter(|release| is_stable_cli_release(&release.version))
        .filter(|release| is_newer_cli_release(env!("CARGO_PKG_VERSION"), &release.version))
        .filter(|release| cli_asset_for_release(release, target).is_some());

    newer_releases
        .clone()
        .find(|release| {
            self_update::version::bump_is_compatible(env!("CARGO_PKG_VERSION"), &release.version)
                .unwrap_or(false)
        })
        .or_else(|| newer_releases.into_iter().next())
        .cloned()
}

fn is_stable_cli_release(version: &str) -> bool {
    !version
        .trim_start_matches('v')
        .split('+')
        .next()
        .unwrap_or(version)
        .contains('-')
}

fn is_newer_cli_release(current_version: &str, candidate_version: &str) -> bool {
    let Some(current) = parse_cli_semver(current_version) else {
        return false;
    };
    let Some(candidate) = parse_cli_semver(candidate_version) else {
        return false;
    };
    candidate > current
}

fn parse_cli_semver(version: &str) -> Option<(u64, u64, u64)> {
    let version = version.trim_start_matches('v');
    let version = version.split(['-', '+']).next()?;
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn cli_asset_for_release(
    release: &self_update::update::Release,
    target: &str,
) -> Option<self_update::update::ReleaseAsset> {
    let target_substrings = cli_asset_target_substrings(target);
    let explicit_cli_label = target.starts_with("cli-linux-") || target.starts_with("cli-windows-");
    release
        .assets
        .iter()
        .find(|asset| {
            asset.name.contains(CLI_UPDATE_IDENTIFIER)
                && target_substrings
                    .iter()
                    .any(|candidate| asset.name.contains(candidate))
        })
        .cloned()
        .or_else(|| {
            if explicit_cli_label {
                None
            } else {
                release.asset_for(target, Some(CLI_UPDATE_IDENTIFIER))
            }
        })
}

fn cli_asset_target_substrings(target: &str) -> Vec<String> {
    match target {
        "cli-linux-x86_64" => vec!["x86_64-unknown-linux-gnu".to_string(), target.to_string()],
        "cli-linux-aarch64" => vec!["aarch64-unknown-linux-gnu".to_string(), target.to_string()],
        "cli-windows-x86_64" => vec!["x86_64-pc-windows-msvc".to_string(), target.to_string()],
        "cli-windows-aarch64" => vec!["aarch64-pc-windows-msvc".to_string(), target.to_string()],
        _ => vec![target.to_string()],
    }
}

fn download_cli_asset(download_url: &str) -> Result<Vec<u8>, String> {
    let mut payload = Vec::new();
    self_update::Download::from_url(download_url)
        .set_header(
            reqwest::header::ACCEPT,
            "application/octet-stream".parse().unwrap(),
        )
        .download_to(&mut payload)
        .map_err(|err| format!("failed to download CLI update: {err}"))?;
    Ok(payload)
}

fn verify_cli_payload_signature(
    payload: &[u8],
    asset_name: &str,
    target: &str,
) -> Result<(), String> {
    let keys = zipsign_api::verify::collect_keys([Ok(CLI_UPDATE_VERIFYING_KEY)])
        .map_err(|err| format!("failed to load CLI update signing key: {err}"))?;
    let context = Some(asset_name.as_bytes());
    let mut reader = Cursor::new(payload);

    if is_cli_linux_target(target) {
        zipsign_api::verify::verify_tar(&mut reader, &keys, context)
            .map_err(|err| format!("failed to verify CLI update signature: {err}"))?;
        return Ok(());
    }
    if is_cli_windows_target(target) {
        zipsign_api::verify::verify_zip(&mut reader, &keys, context)
            .map_err(|err| format!("failed to verify CLI update signature: {err}"))?;
        return Ok(());
    }

    Err(format!("unsupported CLI update target: {target}"))
}

pub fn maybe_auto_update() {
    if std::env::var("FINI_DISABLE_AUTO_UPDATE").ok().as_deref() == Some("1") {
        return;
    }

    let Some(state_dir) = dirs::state_dir().or_else(dirs::data_local_dir) else {
        return;
    };
    let stamp = state_dir.join("fini").join("cli-update-last-check");
    maybe_auto_update_with_stamp(&stamp, run_update);
}

fn maybe_auto_update_with_stamp(
    stamp: &Path,
    run_update: impl FnOnce(UpdateOptions) -> Result<Value, String>,
) {
    let due = fs::metadata(stamp)
        .and_then(|metadata| metadata.modified())
        .and_then(|modified| {
            SystemTime::now()
                .duration_since(modified)
                .map_err(std::io::Error::other)
        })
        .map(|elapsed| elapsed >= Duration::from_secs(24 * 60 * 60))
        .unwrap_or(true);
    if !due {
        return;
    }

    let _ = run_update(UpdateOptions {
        dry_run: false,
        endpoint: None,
        pubkey: None,
        target: None,
        executable_path: None,
    });
    if let Some(parent) = stamp.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(stamp, b"checked\n");
}

fn install_verified_cli_payload(
    payload: &[u8],
    target: &str,
    expected_version: &str,
    executable_path: &Path,
) -> Result<(), String> {
    let metadata = fs::metadata(executable_path)
        .map_err(|err| format!("failed to inspect current executable: {err}"))?;
    if !metadata.is_file() {
        return Err(format!(
            "current executable is not a regular file: {}",
            executable_path.display()
        ));
    }

    let parent = executable_path.parent().ok_or_else(|| {
        format!(
            "failed to resolve executable directory for {}",
            executable_path.display()
        )
    })?;
    let mode = executable_mode(&metadata);
    let candidate_bytes = extract_cli_from_archive(payload, target)?;
    validate_candidate_architecture(&candidate_bytes, target)?;

    let staging_dir = tempfile::Builder::new()
        .prefix(".fini-update-")
        .tempdir_in(parent)
        .map_err(|err| format!("failed to create update staging directory: {err}"))?;
    let candidate_path = staging_dir.path().join(candidate_filename(target));
    fs::write(&candidate_path, &candidate_bytes)
        .map_err(|err| format!("failed to stage update candidate: {err}"))?;
    set_executable_mode(&candidate_path, mode)?;
    validate_candidate_version(&candidate_path, expected_version)?;

    if is_cli_windows_target(target) && should_defer_windows_self_replacement(executable_path) {
        let pending_path = windows_pending_replacement_path(executable_path);
        fs::copy(&candidate_path, &pending_path)
            .map_err(|err| format!("failed to stage Windows replacement helper payload: {err}"))?;
        schedule_windows_self_replacement(&pending_path, executable_path)?;
        return Ok(());
    }

    let backup_path = parent.join(format!(
        ".{}.fini-update-backup",
        executable_path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("fini")
    ));
    if backup_path.exists() {
        fs::remove_file(&backup_path)
            .map_err(|err| format!("failed to remove stale update backup: {err}"))?;
    }

    fs::rename(executable_path, &backup_path)
        .map_err(|err| format!("failed to preserve current executable before update: {err}"))?;

    let replace_result = (|| {
        fs::rename(&candidate_path, executable_path)
            .map_err(|err| format!("failed to install update candidate: {err}"))?;
        set_executable_mode(executable_path, mode)?;
        validate_candidate_version(executable_path, expected_version)?;
        Ok::<(), String>(())
    })();

    match replace_result {
        Ok(()) => {
            fs::remove_file(&backup_path)
                .map_err(|err| format!("updated but failed to remove backup: {err}"))?;
            Ok(())
        }
        Err(err) => {
            let _ = fs::remove_file(executable_path);
            fs::rename(&backup_path, executable_path).map_err(|restore_err| {
                format!("{err}; additionally failed to restore previous executable: {restore_err}")
            })?;
            Err(err)
        }
    }
}

fn extract_cli_from_archive(payload: &[u8], target: &str) -> Result<Vec<u8>, String> {
    if is_cli_linux_target(target) {
        return extract_fini_from_tar_gz(payload);
    }
    if is_cli_windows_target(target) {
        return extract_fini_from_zip(payload);
    }
    Err(format!("unsupported CLI update target: {target}"))
}

fn extract_fini_from_tar_gz(payload: &[u8]) -> Result<Vec<u8>, String> {
    let decoder = GzDecoder::new(Cursor::new(payload));
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|err| format!("invalid CLI tar.gz artifact: {err}"))?;

    for entry in entries {
        let mut entry = entry.map_err(|err| format!("invalid CLI tar entry: {err}"))?;
        let path = entry
            .path()
            .map_err(|err| format!("invalid CLI tar entry path: {err}"))?;
        if path.as_ref() == Path::new("fini") {
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .map_err(|err| format!("failed to read fini from CLI archive: {err}"))?;
            return Ok(bytes);
        }
    }

    Err("CLI archive does not contain fini at the top level".to_string())
}

fn extract_fini_from_zip(payload: &[u8]) -> Result<Vec<u8>, String> {
    let reader = Cursor::new(payload);
    let mut archive =
        zip::ZipArchive::new(reader).map_err(|err| format!("invalid CLI zip artifact: {err}"))?;
    for name in ["fini.exe", "fini"] {
        if let Ok(mut file) = archive.by_name(name) {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)
                .map_err(|err| format!("failed to read {name} from CLI archive: {err}"))?;
            return Ok(bytes);
        }
    }

    Err("CLI archive does not contain fini.exe at the top level".to_string())
}

fn validate_candidate_architecture(bytes: &[u8], target: &str) -> Result<(), String> {
    let expected = expected_architecture(target)?;
    let actual = detect_architecture(bytes)
        .ok_or_else(|| "failed to identify CLI candidate architecture".to_string())?;
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "CLI candidate architecture mismatch: expected {expected}, found {actual}"
        ))
    }
}

fn expected_architecture(target: &str) -> Result<&'static str, String> {
    if target_architecture(target) == Some("x86_64") {
        Ok("x86_64")
    } else if target_architecture(target) == Some("aarch64") {
        Ok("aarch64")
    } else {
        Err(format!(
            "unsupported CLI update architecture target: {target}"
        ))
    }
}

fn detect_architecture(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 20 && bytes.starts_with(b"\x7fELF") {
        let machine = u16::from_le_bytes([bytes[18], bytes[19]]);
        return match machine {
            62 => Some("x86_64"),
            183 => Some("aarch64"),
            _ => None,
        };
    }

    if bytes.len() >= 0x40 && bytes.starts_with(b"MZ") {
        let pe_offset =
            u32::from_le_bytes([bytes[0x3c], bytes[0x3d], bytes[0x3e], bytes[0x3f]]) as usize;
        if bytes.len() >= pe_offset + 6 && &bytes[pe_offset..pe_offset + 4] == b"PE\0\0" {
            let machine = u16::from_le_bytes([bytes[pe_offset + 4], bytes[pe_offset + 5]]);
            return match machine {
                0x8664 => Some("x86_64"),
                0xaa64 => Some("aarch64"),
                _ => None,
            };
        }
    }

    None
}

fn validate_candidate_version(path: &Path, expected_version: &str) -> Result<(), String> {
    let output = Command::new(path)
        .arg("--version")
        .output()
        .map_err(|err| format!("failed to run update candidate --version: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "update candidate --version failed with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let actual = stdout.trim();
    let expected = format!("fini {expected_version}");
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "update candidate version mismatch: expected '{expected}', got '{actual}'"
        ))
    }
}

fn candidate_filename(target: &str) -> &'static str {
    if is_cli_windows_target(target) {
        "fini.exe"
    } else {
        "fini"
    }
}

fn is_cli_linux_target(target: &str) -> bool {
    target.starts_with("cli-linux-") || target.ends_with("-unknown-linux-gnu")
}

fn is_cli_windows_target(target: &str) -> bool {
    target.starts_with("cli-windows-") || target.ends_with("-pc-windows-msvc")
}

fn target_architecture(target: &str) -> Option<&'static str> {
    if target.ends_with("-x86_64") || target.starts_with("x86_64-") {
        Some("x86_64")
    } else if target.ends_with("-aarch64") || target.starts_with("aarch64-") {
        Some("aarch64")
    } else {
        None
    }
}

#[cfg(windows)]
fn should_defer_windows_self_replacement(executable_path: &Path) -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|current| same_file_path(&current, executable_path).ok())
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn should_defer_windows_self_replacement(_executable_path: &Path) -> bool {
    false
}

#[cfg(windows)]
fn same_file_path(left: &Path, right: &Path) -> Result<bool, std::io::Error> {
    Ok(left.canonicalize()? == right.canonicalize()?)
}

fn windows_pending_replacement_path(executable_path: &Path) -> PathBuf {
    let file_name = executable_path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("fini.exe");
    executable_path.with_file_name(format!(".{file_name}.fini-update-pending"))
}

#[cfg(windows)]
fn schedule_windows_self_replacement(
    pending_path: &Path,
    executable_path: &Path,
) -> Result<(), String> {
    let script = windows_self_replacement_script(pending_path, executable_path);
    Command::new("cmd")
        .args(["/C", "start", "", "/MIN", "cmd", "/C", &script])
        .spawn()
        .map_err(|err| format!("failed to start Windows replacement helper: {err}"))?;
    Ok(())
}

#[cfg(not(windows))]
fn schedule_windows_self_replacement(
    _pending_path: &Path,
    _executable_path: &Path,
) -> Result<(), String> {
    Ok(())
}

fn windows_self_replacement_script(pending_path: &Path, executable_path: &Path) -> String {
    let pending = windows_cmd_quote(pending_path);
    let target = windows_cmd_quote(executable_path);
    format!(
        "for /L %i in (1,1,60) do @(move /Y {pending} {target} >NUL 2>NUL && exit /B 0 || ping 127.0.0.1 -n 2 >NUL)"
    )
}

fn windows_cmd_quote(path: &Path) -> String {
    let escaped = path.display().to_string().replace('\"', "\"\"");
    format!("\"{escaped}\"")
}

#[cfg(unix)]
fn executable_mode(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn executable_mode(_metadata: &fs::Metadata) -> u32 {
    0
}

#[cfg(unix)]
fn set_executable_mode(path: &Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|err| format!("failed to set executable permissions: {err}"))
}

#[cfg(not(unix))]
fn set_executable_mode(_path: &Path, _mode: u32) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn extracts_only_top_level_fini_from_linux_archive() {
        let dir = tempfile::tempdir().expect("tempdir");
        let payload = make_tar_gz(&[("nested/fini", b"wrong"), ("fini", b"right")]);
        let bytes = extract_cli_from_archive(&payload, current_cli_linux_target())
            .expect("top-level fini extracted");

        assert_eq!(bytes, b"right");
        assert!(!dir.path().join("nested").exists());
    }

    #[test]
    fn rejects_wrong_architecture() {
        let mut elf = vec![0; 64];
        elf[0..4].copy_from_slice(b"\x7fELF");
        elf[18..20].copy_from_slice(&183u16.to_le_bytes());

        let err = validate_candidate_architecture(&elf, "cli-linux-x86_64")
            .expect_err("wrong arch should fail");

        assert!(err.contains("architecture mismatch"));
    }

    #[test]
    fn accepts_self_update_rust_targets_for_cli_installation() {
        let linux_payload = make_tar_gz(&[("fini", b"linux")]);
        let linux_bytes = extract_cli_from_archive(&linux_payload, "x86_64-unknown-linux-gnu")
            .expect("Rust Linux target extracts tar.gz CLI payload");
        assert_eq!(linux_bytes, b"linux");
        assert_eq!(
            expected_architecture("x86_64-unknown-linux-gnu").expect("linux architecture"),
            "x86_64"
        );

        let windows_payload = make_zip(&[("fini.exe", b"windows")]);
        let windows_bytes = extract_cli_from_archive(&windows_payload, "x86_64-pc-windows-msvc")
            .expect("Rust Windows target extracts zip CLI payload");
        assert_eq!(windows_bytes, b"windows");
        assert_eq!(
            expected_architecture("x86_64-pc-windows-msvc").expect("windows architecture"),
            "x86_64"
        );
        assert_eq!(candidate_filename("x86_64-pc-windows-msvc"), "fini.exe");
    }

    #[test]
    fn keeps_cli_target_labels_supported_for_cli_installation() {
        assert!(is_cli_linux_target("cli-linux-x86_64"));
        assert!(is_cli_windows_target("cli-windows-aarch64"));
        assert_eq!(
            expected_architecture("cli-windows-aarch64").expect("cli architecture"),
            "aarch64"
        );
        assert_eq!(candidate_filename("cli-windows-aarch64"), "fini.exe");
    }

    #[test]
    #[cfg(unix)]
    fn installs_valid_linux_archive_atomically_and_preserves_mode() {
        let dir = tempfile::tempdir().expect("tempdir");
        let current = dir.path().join("fini");
        let old = compile_version_binary(dir.path(), "old", "1.0.0");
        fs::copy(&old, &current).expect("copy old binary");
        set_executable_mode(&current, 0o755).expect("chmod old");
        let new = compile_version_binary(dir.path(), "new", "1.2.0");
        let payload = make_tar_gz_from_file("fini", &new);

        install_verified_cli_payload(&payload, current_cli_linux_target(), "1.2.0", &current)
            .expect("update installed");

        validate_candidate_version(&current, "1.2.0").expect("new binary runs");
        assert_eq!(
            fs::metadata(&current)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        assert!(!dir.path().join(".fini.fini-update-backup").exists());
    }

    #[test]
    fn windows_replacement_helper_uses_persistent_pending_payload() {
        let executable = PathBuf::from("/updates/fini.exe");
        let pending = windows_pending_replacement_path(&executable);
        let script = windows_self_replacement_script(&pending, &executable);

        assert_eq!(
            pending,
            PathBuf::from("/updates/.fini.exe.fini-update-pending")
        );
        assert!(script.contains("move /Y"));
        assert!(script.contains(".fini.exe.fini-update-pending"));
        assert!(script.contains("fini.exe"));
        assert!(script.contains(" || ping 127.0.0.1 -n 2 >NUL)"));
        assert!(!script.contains(") & ping 127.0.0.1"));
    }

    #[test]
    fn automatic_update_stamps_failed_attempts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let stamp = dir.path().join("state").join("cli-update-last-check");

        maybe_auto_update_with_stamp(&stamp, |_| Err("offline".to_string()));

        assert_eq!(
            fs::read_to_string(&stamp).expect("stamp written"),
            "checked\n"
        );
    }

    #[test]
    fn automatic_update_skips_recent_stamp() {
        let dir = tempfile::tempdir().expect("tempdir");
        let stamp = dir.path().join("cli-update-last-check");
        fs::write(&stamp, b"checked\n").expect("write stamp");

        maybe_auto_update_with_stamp(&stamp, |_| panic!("recent stamp should skip update check"));
    }

    #[test]
    fn compatible_release_selection_skips_older_releases() {
        let releases = vec![release_with_cli_asset(
            "0.1.45",
            "fini-0.1.45-linux-x86_64-unknown-linux-gnu-cli.tar.gz",
        )];

        assert!(compatible_cli_release(&releases, "cli-linux-x86_64").is_none());
    }

    #[test]
    fn compatible_release_selection_keeps_newer_releases() {
        let releases = vec![release_with_cli_asset(
            "0.1.47",
            "fini-0.1.47-linux-x86_64-unknown-linux-gnu-cli.tar.gz",
        )];

        assert_eq!(
            compatible_cli_release(&releases, "cli-linux-x86_64")
                .expect("newer release selected")
                .version,
            "0.1.47"
        );
    }

    #[test]
    fn compatible_release_selection_skips_prereleases_for_stable_updates() {
        let releases = vec![
            release_with_cli_asset(
                "0.1.47-rc.1",
                "fini-0.1.47-rc.1-linux-x86_64-unknown-linux-gnu-cli.tar.gz",
            ),
            release_with_cli_asset(
                "0.1.46",
                "fini-0.1.46-linux-x86_64-unknown-linux-gnu-cli.tar.gz",
            ),
        ];

        assert!(compatible_cli_release(&releases, "cli-linux-x86_64").is_none());
    }

    #[test]
    fn compatible_release_selection_prefers_stable_release_over_newer_prerelease() {
        let releases = vec![
            release_with_cli_asset(
                "0.1.48-rc.1",
                "fini-0.1.48-rc.1-linux-x86_64-unknown-linux-gnu-cli.tar.gz",
            ),
            release_with_cli_asset(
                "0.1.47",
                "fini-0.1.47-linux-x86_64-unknown-linux-gnu-cli.tar.gz",
            ),
        ];

        assert_eq!(
            compatible_cli_release(&releases, "cli-linux-x86_64")
                .expect("stable release selected")
                .version,
            "0.1.47"
        );
    }

    #[test]
    fn compatible_release_selection_falls_back_to_newer_incompatible_releases() {
        let releases = vec![release_with_cli_asset(
            "0.2.0",
            "fini-0.2.0-linux-x86_64-unknown-linux-gnu-cli.tar.gz",
        )];

        assert_eq!(
            compatible_cli_release(&releases, "cli-linux-x86_64")
                .expect("future incompatible release selected")
                .version,
            "0.2.0"
        );
    }

    #[test]
    fn compatible_release_selection_prefers_compatible_newer_release() {
        let releases = vec![
            release_with_cli_asset(
                "0.2.0",
                "fini-0.2.0-linux-x86_64-unknown-linux-gnu-cli.tar.gz",
            ),
            release_with_cli_asset(
                "0.1.47",
                "fini-0.1.47-linux-x86_64-unknown-linux-gnu-cli.tar.gz",
            ),
        ];

        assert_eq!(
            compatible_cli_release(&releases, "cli-linux-x86_64")
                .expect("compatible newer release selected first")
                .version,
            "0.1.47"
        );
    }

    #[test]
    fn compatible_release_selection_accepts_cli_label_for_rust_target_asset() {
        let release = release_with_cli_asset(
            "0.1.47",
            "fini-0.1.47-linux-x86_64-unknown-linux-gnu-cli.tar.gz",
        );
        let releases = vec![release.clone()];

        let selected =
            compatible_cli_release(&releases, "cli-linux-x86_64").expect("release selected");
        let asset = cli_asset_for_release(&selected, "cli-linux-x86_64")
            .expect("Rust-targeted CLI asset selected for CLI label");

        assert_eq!(selected.version, "0.1.47");
        assert_eq!(asset.name, release.assets[0].name);
    }

    #[test]
    fn compatible_release_selection_skips_releases_without_matching_cli_asset() {
        let releases = vec![
            release_with_asset("0.1.47", "fini-0.1.47-x86_64.AppImage"),
            release_with_cli_asset(
                "0.1.48",
                "fini-0.1.48-linux-x86_64-unknown-linux-gnu-cli.tar.gz",
            ),
        ];

        assert_eq!(
            compatible_cli_release(&releases, "cli-linux-x86_64")
                .expect("CLI release selected")
                .version,
            "0.1.48"
        );
    }

    #[test]
    fn cli_label_asset_selection_does_not_fall_back_to_host_asset() {
        let release = release_with_cli_asset(
            "0.1.47",
            "fini-0.1.47-linux-x86_64-unknown-linux-gnu-cli.tar.gz",
        );

        assert!(cli_asset_for_release(&release, "cli-windows-x86_64").is_none());
    }

    #[test]
    #[cfg(unix)]
    fn invalid_artifact_keeps_previous_executable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let current = dir.path().join("fini");
        let old = compile_version_binary(dir.path(), "old-invalid", "1.0.0");
        fs::copy(&old, &current).expect("copy old binary");
        set_executable_mode(&current, 0o755).expect("chmod old");
        let payload = make_tar_gz(&[("not-fini", b"nope")]);

        let err =
            install_verified_cli_payload(&payload, current_cli_linux_target(), "1.2.0", &current)
                .expect_err("invalid artifact should fail");

        assert!(err.contains("does not contain fini"));
        validate_candidate_version(&current, "1.0.0").expect("old binary still runs");
    }

    #[test]
    #[cfg(unix)]
    fn failed_post_download_validation_restores_previous_executable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let current = dir.path().join("fini");
        let old = compile_version_binary(dir.path(), "old-rollback", "1.0.0");
        fs::copy(&old, &current).expect("copy old binary");
        set_executable_mode(&current, 0o755).expect("chmod old");
        let bad = compile_version_binary(dir.path(), "bad-version", "9.9.9");
        let payload = make_tar_gz_from_file("fini", &bad);

        let err =
            install_verified_cli_payload(&payload, current_cli_linux_target(), "1.2.0", &current)
                .expect_err("version mismatch should fail");

        assert!(err.contains("version mismatch"));
        validate_candidate_version(&current, "1.0.0").expect("old binary restored");
    }

    fn current_cli_linux_target() -> &'static str {
        #[cfg(target_arch = "x86_64")]
        {
            "cli-linux-x86_64"
        }
        #[cfg(target_arch = "aarch64")]
        {
            "cli-linux-aarch64"
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            "cli-linux-x86_64"
        }
    }

    fn compile_version_binary(dir: &Path, name: &str, version: &str) -> PathBuf {
        let source = dir.join(format!("{name}.rs"));
        let binary = dir.join(name);
        fs::write(
            &source,
            format!(
                "fn main() {{ if std::env::args().nth(1).as_deref() == Some(\"--version\") {{ println!(\"fini {version}\"); }} }}"
            ),
        )
        .expect("write fixture source");
        let status = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .status()
            .expect("run rustc");
        assert!(status.success(), "fixture rustc failed");
        binary
    }

    fn release_with_cli_asset(version: &str, asset_name: &str) -> self_update::update::Release {
        release_with_asset(version, asset_name)
    }

    fn release_with_asset(version: &str, asset_name: &str) -> self_update::update::Release {
        self_update::update::Release {
            version: version.to_string(),
            assets: vec![self_update::update::ReleaseAsset {
                download_url: format!("https://example.test/{asset_name}"),
                name: asset_name.to_string(),
            }],
            ..Default::default()
        }
    }

    fn make_tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        {
            let mut archive = tar::Builder::new(&mut gz);
            for (path, bytes) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_size(bytes.len() as u64);
                header.set_mode(0o755);
                header.set_cksum();
                archive
                    .append_data(&mut header, *path, Cursor::new(*bytes))
                    .expect("append tar entry");
            }
            archive.finish().expect("finish tar");
        }
        gz.finish().expect("finish gzip")
    }

    fn make_tar_gz_from_file(path: &str, source: &Path) -> Vec<u8> {
        let bytes = fs::read(source).expect("read source binary");
        make_tar_gz(&[(path, bytes.as_slice())])
    }

    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut archive = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default();
        for (path, bytes) in entries {
            archive.start_file(*path, options).expect("start zip file");
            std::io::Write::write_all(&mut archive, bytes).expect("write zip file");
        }
        archive.finish().expect("finish zip").into_inner()
    }
}
