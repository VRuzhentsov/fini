use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallChannel {
    AppImage,
    Flatpak,
    LinuxPackage,
    DevBuild,
    Standalone,
    Unknown,
}

impl InstallChannel {
    pub fn label(self) -> &'static str {
        match self {
            Self::AppImage => "AppImage",
            Self::Flatpak => "Flatpak",
            Self::LinuxPackage => "Linux package",
            Self::DevBuild => "development build",
            Self::Standalone => "standalone build",
            Self::Unknown => "unknown install channel",
        }
    }
}

pub fn detect_install_channel() -> InstallChannel {
    if let Some(channel) = std::env::var("FINI_INSTALL_CHANNEL")
        .ok()
        .and_then(|value| parse_install_channel(&value))
    {
        return channel;
    }

    if std::env::var_os("APPIMAGE").is_some() {
        return InstallChannel::AppImage;
    }

    if std::env::var_os("FLATPAK_ID").is_some() {
        return InstallChannel::Flatpak;
    }

    if cfg!(debug_assertions) {
        return InstallChannel::DevBuild;
    }

    match std::env::current_exe() {
        Ok(path) if looks_like_system_package_path(&path) => InstallChannel::LinuxPackage,
        Ok(_) => InstallChannel::Standalone,
        Err(_) => InstallChannel::Unknown,
    }
}

pub fn unsupported_schema_guidance() -> String {
    unsupported_schema_guidance_for(detect_install_channel())
}

pub fn unsupported_schema_guidance_for(channel: InstallChannel) -> String {
    let next_step = match channel {
        InstallChannel::AppImage => {
            "Download the latest Fini AppImage, replace the current AppImage, then reopen Fini."
        }
        InstallChannel::Flatpak => {
            "Run `flatpak update` for Fini, then reopen the app or rerun the command."
        }
        InstallChannel::LinuxPackage => {
            "Update Fini through your system package manager, then reopen the app or rerun the command."
        }
        InstallChannel::DevBuild => {
            "Rebuild or switch to a Fini binary that includes the database migration, then rerun the command."
        }
        InstallChannel::Standalone => {
            "Install the latest Fini binary for this machine, then reopen the app or rerun the command."
        }
        InstallChannel::Unknown => {
            "Install the latest Fini release for this machine, then reopen the app or rerun the command."
        }
    };

    format!(
        "Update required ({channel}). {next_step} Fini cannot continue with this database until the running binary supports its schema.",
        channel = channel.label()
    )
}

fn parse_install_channel(value: &str) -> Option<InstallChannel> {
    match value.trim().to_ascii_lowercase().as_str() {
        "appimage" => Some(InstallChannel::AppImage),
        "flatpak" => Some(InstallChannel::Flatpak),
        "deb" | "rpm" | "package" | "linux-package" => Some(InstallChannel::LinuxPackage),
        "dev" | "development" | "dev-build" => Some(InstallChannel::DevBuild),
        "standalone" => Some(InstallChannel::Standalone),
        "unknown" => Some(InstallChannel::Unknown),
        _ => None,
    }
}

fn looks_like_system_package_path(path: &Path) -> bool {
    path.starts_with("/usr/bin") || path.starts_with("/usr/local/bin") || path.starts_with("/opt")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guidance_mentions_appimage_recovery() {
        let guidance = unsupported_schema_guidance_for(InstallChannel::AppImage);

        assert!(guidance.contains("Update required (AppImage)"));
        assert!(guidance.contains("latest Fini AppImage"));
        assert!(guidance.contains("cannot continue"));
    }

    #[test]
    fn install_channel_override_supports_package_aliases() {
        assert_eq!(
            parse_install_channel("deb"),
            Some(InstallChannel::LinuxPackage)
        );
        assert_eq!(
            parse_install_channel("rpm"),
            Some(InstallChannel::LinuxPackage)
        );
        assert_eq!(
            parse_install_channel("linux-package"),
            Some(InstallChannel::LinuxPackage)
        );
    }

    #[test]
    fn invalid_install_channel_override_is_ignored() {
        assert_eq!(parse_install_channel("sideways"), None);
    }
}
