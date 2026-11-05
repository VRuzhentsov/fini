#!/usr/bin/env bash
# Prepare the tools that Tauri's AppImage bundler uses, in its tools folder.
# Tauri downloads each missing tool itself, with no retry: one DNS fault on the
# runner failed the Linux x64 build of v0.3.18 ("failed to lookup address
# information" for AppRun-x86_64). Here each download has retries, and Tauri
# then finds the files and downloads nothing. The URLs and names are the ones
# of tauri-bundler `prepare_tools` (crates/tauri-bundler/src/bundle/linux/appimage/linuxdeploy.rs).
#
# The GTK plugin comes from a reviewed upstream revision, not from Tauri's
# unpinned branch, and gets Fini's host-Wayland runtime policy appended.
set -euo pipefail

plugin_revision='b5eb8d05b4c0ed40107fe2158c5d8527f94568ef'
plugin_sha256='cb379f9b0733e9ad9f8bd78f8c2fa038aef2478523bb7d4c8e64ff6a1ea3501a'
plugin_url="https://raw.githubusercontent.com/tauri-apps/linuxdeploy-plugin-gtk/${plugin_revision}/linuxdeploy-plugin-gtk.sh"
tools_dir="${XDG_CACHE_HOME:-"$HOME/.cache"}/tauri"
plugin_path="$tools_dir/linuxdeploy-plugin-gtk.sh"
arch="$(uname -m)"

download() {
  curl --fail --location --silent --show-error --retry 5 --retry-all-errors --retry-delay 5 "$1" --output "$2"
  chmod +x "$2"
}

mkdir -p "$tools_dir"
download "https://github.com/tauri-apps/binary-releases/releases/download/apprun-old/AppRun-${arch}" "$tools_dir/AppRun-${arch}"
download "https://github.com/tauri-apps/binary-releases/releases/download/linuxdeploy/linuxdeploy-${arch}.AppImage" "$tools_dir/linuxdeploy-${arch}.AppImage"
download "https://raw.githubusercontent.com/tauri-apps/linuxdeploy-plugin-gstreamer/master/linuxdeploy-plugin-gstreamer.sh" "$tools_dir/linuxdeploy-plugin-gstreamer.sh"
download "https://github.com/linuxdeploy/linuxdeploy-plugin-appimage/releases/download/continuous/linuxdeploy-plugin-appimage-${arch}.AppImage" "$tools_dir/linuxdeploy-plugin-appimage.AppImage"
download "$plugin_url" "$plugin_path"
printf '%s  %s\n' "$plugin_sha256" "$plugin_path" | sha256sum --check --status

cat >> "$plugin_path" <<'EOF'

# Fini AppImage portability policy:
# Wayland client ABI must come from the run host so it matches that host's
# Mesa/EGL implementation. linuxdeploy has already assembled the AppDir here;
# removing these entries before its output plugin creates the final AppImage.
for fini_host_wayland_library in \
  libwayland-client.so.0 \
  libwayland-cursor.so.0 \
  libwayland-egl.so.1 \
  libwayland-server.so.0
do
  find "$APPDIR/usr/lib" \( -type f -o -type l \) \
    -name "$fini_host_wayland_library" -delete
done
EOF

chmod +x "$plugin_path"
printf 'Prepared pinned GTK linuxdeploy plugin at %s\n' "$plugin_path"
