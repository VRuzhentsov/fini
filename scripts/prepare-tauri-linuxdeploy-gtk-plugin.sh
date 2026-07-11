#!/usr/bin/env bash
# Prepare the exact GTK linuxdeploy plugin consumed by Tauri's AppImage bundler.
# Tauri downloads this plugin from an unpinned branch by default; CI instead uses
# a reviewed upstream revision and appends Fini's host-Wayland runtime policy.
set -euo pipefail

plugin_revision='b5eb8d05b4c0ed40107fe2158c5d8527f94568ef'
plugin_sha256='cb379f9b0733e9ad9f8bd78f8c2fa038aef2478523bb7d4c8e64ff6a1ea3501a'
plugin_url="https://raw.githubusercontent.com/tauri-apps/linuxdeploy-plugin-gtk/${plugin_revision}/linuxdeploy-plugin-gtk.sh"
tools_dir="${XDG_CACHE_HOME:-"$HOME/.cache"}/tauri"
plugin_path="$tools_dir/linuxdeploy-plugin-gtk.sh"

mkdir -p "$tools_dir"
curl --fail --location --silent --show-error "$plugin_url" --output "$plugin_path"
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
