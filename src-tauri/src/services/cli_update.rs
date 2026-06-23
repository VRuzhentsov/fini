use serde_json::{json, Value};
use std::path::PathBuf;
use tauri_plugin_updater::UpdaterExt;
use url::Url;

const DEFAULT_UPDATE_ENDPOINT: &str =
    "https://github.com/VRuzhentsov/fini/releases/latest/download/latest-cli.json";
const COMPILED_UPDATE_PUBKEY: Option<&str> = option_env!("FINI_TAURI_UPDATER_PUBKEY");

#[derive(Debug, Clone)]
pub struct UpdateOptions {
    pub dry_run: bool,
    pub endpoint: Option<String>,
    pub pubkey: Option<String>,
    pub target: Option<String>,
    pub executable_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliUpdateConfig {
    endpoint: Url,
    pubkey: String,
    target: String,
    executable_path: PathBuf,
}

pub fn run_update(options: UpdateOptions) -> Result<Value, String> {
    let config = resolve_update_config(options.clone())?;
    let app = tauri::Builder::default()
        .plugin(
            tauri_plugin_updater::Builder::new()
                .pubkey(config.pubkey.clone())
                .target(config.target.clone())
                .build(),
        )
        .build(tauri::generate_context!())
        .map_err(|err| format!("failed to initialize Tauri updater: {err}"))?;

    let app_handle = app.handle().clone();
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|err| format!("failed to create update runtime: {err}"))?;
    runtime.block_on(async move {
        let mut builder = app_handle
            .updater_builder()
            .target(config.target.clone())
            .pubkey(config.pubkey.clone())
            .endpoints(vec![config.endpoint.clone()])
            .map_err(|err| format!("failed to configure updater endpoint: {err}"))?
            .executable_path(&config.executable_path);

        if let Some(token) = github_token_for_endpoint(&config.endpoint) {
            builder = builder
                .header("Authorization", format!("Bearer {token}"))
                .map_err(|err| format!("failed to configure updater auth header: {err}"))?;
        }

        let updater = builder
            .build()
            .map_err(|err| format!("failed to build updater: {err}"))?;
        let update = updater
            .check()
            .await
            .map_err(|err| format!("failed to check for updates: {err}"))?;

        let Some(update) = update else {
            return Ok(json!({
                "current_version": env!("CARGO_PKG_VERSION"),
                "target_version": env!("CARGO_PKG_VERSION"),
                "endpoint": config.endpoint,
                "target": config.target,
                "executable_path": config.executable_path,
                "dry_run": options.dry_run,
                "already_current": true,
                "updated": false,
            }));
        };

        let result = json!({
            "current_version": update.current_version,
            "target_version": update.version,
            "endpoint": config.endpoint,
            "target": update.target,
            "download_url": update.download_url,
            "executable_path": config.executable_path,
            "dry_run": options.dry_run,
            "already_current": false,
            "updated": !options.dry_run,
        });

        if !options.dry_run {
            update
                .download_and_install(|_, _| {}, || {})
                .await
                .map_err(|err| format!("failed to install update: {err}"))?;
        }

        Ok(result)
    })
}

fn resolve_update_config(options: UpdateOptions) -> Result<CliUpdateConfig, String> {
    let endpoint = options
        .endpoint
        .or_else(|| std::env::var("FINI_UPDATE_ENDPOINT").ok())
        .unwrap_or_else(|| DEFAULT_UPDATE_ENDPOINT.to_string())
        .parse::<Url>()
        .map_err(|err| format!("invalid updater endpoint: {err}"))?;
    let pubkey = options
        .pubkey
        .or_else(|| std::env::var("FINI_UPDATE_PUBKEY").ok())
        .or_else(|| COMPILED_UPDATE_PUBKEY.map(str::to_string))
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            "missing Tauri updater public key; set FINI_UPDATE_PUBKEY or build with FINI_TAURI_UPDATER_PUBKEY".to_string()
        })?;
    let target = options.target.unwrap_or(default_cli_update_target()?);
    let executable_path = options
        .executable_path
        .map(Ok)
        .unwrap_or_else(std::env::current_exe)
        .map_err(|err| format!("failed to resolve current CLI executable: {err}"))?;

    Ok(CliUpdateConfig {
        endpoint,
        pubkey,
        target,
        executable_path,
    })
}

fn default_cli_update_target() -> Result<String, String> {
    tauri_plugin_updater::target()
        .map(|target| format!("cli-{target}"))
        .ok_or_else(|| "CLI updates are not supported on this platform".to_string())
}

fn github_token_for_endpoint(endpoint: &Url) -> Option<String> {
    if !is_trusted_github_update_endpoint(endpoint) {
        return None;
    }

    std::env::var("GH_TOKEN")
        .ok()
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
}

fn is_trusted_github_update_endpoint(endpoint: &Url) -> bool {
    endpoint.scheme() == "https"
        && matches!(endpoint.host_str(), Some("github.com" | "api.github.com"))
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn default_cli_target_prefixes_tauri_target() {
        let target = default_cli_update_target().expect("supported updater target");

        assert!(target.starts_with("cli-"));
        assert_eq!(
            Some(target.trim_start_matches("cli-")),
            tauri_plugin_updater::target().as_deref()
        );
    }

    #[test]
    fn update_config_uses_runtime_options() {
        let executable_path = PathBuf::from("/var/tmp/fini-test-bin");
        let config = resolve_update_config(UpdateOptions {
            dry_run: true,
            endpoint: Some("https://example.test/latest-cli.json".to_string()),
            pubkey: Some("test-public-key".to_string()),
            target: Some("cli-linux-x86_64".to_string()),
            executable_path: Some(executable_path.clone()),
        })
        .expect("resolved update config");

        assert_eq!(
            config.endpoint.as_str(),
            "https://example.test/latest-cli.json"
        );
        assert_eq!(config.pubkey, "test-public-key");
        assert_eq!(config.target, "cli-linux-x86_64");
        assert_eq!(config.executable_path, executable_path);
    }

    #[test]
    fn update_config_requires_pubkey() {
        let result = resolve_update_config(UpdateOptions {
            dry_run: true,
            endpoint: Some("https://example.test/latest-cli.json".to_string()),
            pubkey: Some(" ".to_string()),
            target: Some("cli-linux-x86_64".to_string()),
            executable_path: Some(PathBuf::from("/var/tmp/fini-test-bin")),
        });

        assert!(result
            .expect_err("missing pubkey should fail")
            .contains("missing Tauri updater public key"));
    }

    #[test]
    fn github_token_is_only_used_for_trusted_github_update_endpoints() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        let previous = std::env::var_os("GH_TOKEN");
        std::env::set_var("GH_TOKEN", " test-token ");

        let github_endpoint = Url::parse(
            "https://github.com/VRuzhentsov/fini/releases/latest/download/latest-cli.json",
        )
        .expect("valid GitHub endpoint");
        let api_endpoint = Url::parse("https://api.github.com/repos/VRuzhentsov/fini/releases")
            .expect("valid GitHub API endpoint");
        let custom_endpoint =
            Url::parse("https://updates.example.test/latest-cli.json").expect("valid endpoint");
        let insecure_endpoint =
            Url::parse("http://github.com/VRuzhentsov/fini/latest-cli.json").expect("valid URL");

        assert_eq!(
            github_token_for_endpoint(&github_endpoint).as_deref(),
            Some("test-token")
        );
        assert_eq!(
            github_token_for_endpoint(&api_endpoint).as_deref(),
            Some("test-token")
        );
        assert_eq!(github_token_for_endpoint(&custom_endpoint), None);
        assert_eq!(github_token_for_endpoint(&insecure_endpoint), None);

        match previous {
            Some(value) => std::env::set_var("GH_TOKEN", value),
            None => std::env::remove_var("GH_TOKEN"),
        }
    }
}
