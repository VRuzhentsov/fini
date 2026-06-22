use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const DESKTOP_FILE_NAME: &str = "fini.desktop";
const ICON_NAME: &str = "fini-app";
const APPIMAGE_ENV: &str = "APPIMAGE";
const APPDIR_ENV: &str = "APPDIR";

pub fn self_register_appimage_desktop_entry() -> Result<(), String> {
    let Some(appimage_path) = env::var_os(APPIMAGE_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    else {
        return Ok(());
    };

    let data_home = xdg_data_home()?;
    let desktop_file = data_home.join("applications").join(DESKTOP_FILE_NAME);
    let desktop_entry = desktop_entry_for(&appimage_path);
    write_if_changed(&desktop_file, desktop_entry.as_bytes())?;

    if let Some(appdir) = env::var_os(APPDIR_ENV).map(PathBuf::from) {
        copy_icon_if_available(&appdir, &data_home)?;
    }

    refresh_desktop_caches(&data_home);

    Ok(())
}

fn xdg_data_home() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }

    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".local").join("share"))
        .ok_or_else(|| "HOME is not set; cannot resolve XDG data directory".to_string())
}

fn desktop_entry_for(appimage_path: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Fini\n\
         Comment=Fini\n\
         Exec={} %U\n\
         Icon={ICON_NAME}\n\
         StartupWMClass=fini-app\n\
         Categories=Utility;\n\
         Terminal=false\n",
        quote_desktop_exec_path(appimage_path)
    )
}

fn quote_desktop_exec_path(path: &Path) -> String {
    let escaped = path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn write_if_changed(path: &Path, contents: &[u8]) -> Result<(), String> {
    if fs::read(path)
        .map(|existing| existing == contents)
        .unwrap_or(false)
    {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "failed to create desktop entry directory {}: {err}",
                parent.display()
            )
        })?;
    }

    fs::write(path, contents)
        .map_err(|err| format!("failed to write desktop entry {}: {err}", path.display()))
}

fn copy_icon_if_available(appdir: &Path, data_home: &Path) -> Result<(), String> {
    let candidates = [
        (
            appdir.join("fini-app.png"),
            data_home.join("icons/hicolor/128x128/apps/fini-app.png"),
        ),
        (
            appdir.join("usr/share/icons/hicolor/128x128/apps/fini-app.png"),
            data_home.join("icons/hicolor/128x128/apps/fini-app.png"),
        ),
        (
            appdir.join("usr/share/icons/hicolor/256x256/apps/fini-app.png"),
            data_home.join("icons/hicolor/256x256/apps/fini-app.png"),
        ),
        (
            appdir.join("usr/share/pixmaps/fini-app.png"),
            data_home.join("icons/hicolor/128x128/apps/fini-app.png"),
        ),
    ];

    for (source, destination) in candidates {
        if !source.is_file() {
            continue;
        }

        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "failed to create icon directory {}: {err}",
                    parent.display()
                )
            })?;
        }

        fs::copy(&source, &destination).map_err(|err| {
            format!(
                "failed to copy AppImage icon from {} to {}: {err}",
                source.display(),
                destination.display()
            )
        })?;
        break;
    }

    Ok(())
}

fn refresh_desktop_caches(data_home: &Path) {
    let applications_dir = data_home.join("applications");
    run_cache_command("update-desktop-database", &[applications_dir.as_os_str()]);
    run_cache_command("kbuildsycoca6", &[]);
}

fn run_cache_command(program: &str, args: &[&std::ffi::OsStr]) {
    let _ = Command::new(program).args(args).status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_entry_quotes_appimage_path() {
        let entry = desktop_entry_for(Path::new("/var/tmp/Fini Folder/Fini.AppImage"));

        assert!(entry.contains("Exec=\"/var/tmp/Fini Folder/Fini.AppImage\" %U"));
        assert!(entry.contains("Icon=fini-app"));
        assert!(entry.contains("StartupWMClass=fini-app"));
        assert!(entry.contains("Categories=Utility;"));
    }

    #[test]
    fn write_if_changed_preserves_matching_file() {
        let root = test_dir("write-matching-file");
        let file = root.join("fini.desktop");
        fs::create_dir_all(&root).expect("create test directory");
        fs::write(&file, b"same").expect("seed desktop file");

        write_if_changed(&file, b"same").expect("matching file should be accepted");

        assert_eq!(
            fs::read_to_string(&file).expect("read desktop file"),
            "same"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn copy_icon_uses_appdir_root_icon() {
        let root = test_dir("copy-icon");
        let appdir = root.join("appdir");
        let data_home = root.join("data-home");
        fs::create_dir_all(&appdir).expect("create appdir");
        fs::write(appdir.join("fini-app.png"), b"png").expect("seed icon");

        copy_icon_if_available(&appdir, &data_home).expect("copy icon");

        assert_eq!(
            fs::read(data_home.join("icons/hicolor/128x128/apps/fini-app.png"))
                .expect("read copied icon"),
            b"png"
        );

        let _ = fs::remove_dir_all(root);
    }

    fn test_dir(name: &str) -> PathBuf {
        let path = PathBuf::from("/var/tmp").join(format!(
            "fini-appimage-desktop-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        path
    }
}
