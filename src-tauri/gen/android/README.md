# src-tauri/gen/android/

Auto-generated Android Studio project for the Fini app. Created by `npm run tauri android init` — do not edit generated files manually unless necessary.

## Structure

```
gen/android/
├── app/
│   └── src/main/
│       ├── AndroidManifest.xml          # App permissions and activity declaration
│       ├── java/com/fini/app/
│       │   └── MainActivity.kt          # Android entry point
│       ├── res/
│       │   ├── mipmap-*/                # Launcher icons (all densities)
│       │   ├── values/                  # Colors, strings, themes
│       │   └── xml/file_paths.xml       # File provider paths
│       └── jniLibs/                     # Compiled Rust .so libraries (generated at build time)
├── buildSrc/
│   └── src/main/java/com/fini/app/kotlin/
│       ├── RustPlugin.kt                # Gradle plugin that compiles Rust targets
│       └── BuildTask.kt                 # Gradle task that triggers Tauri CLI
├── gradle/                              # Gradle wrapper
├── build.gradle.kts                     # Root Gradle config
├── app/build.gradle.kts                 # App module Gradle config
└── settings.gradle                      # Project name and module includes
```

## Build

From the repo root:

```bash
npm run tauri android build
```

This compiles the Rust backend for all Android targets (`aarch64`, `armv7`, `i686`, `x86_64`) and assembles the APK/AAB via Gradle.

Output:
- **APK**: `app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk`
- **AAB**: `app/build/outputs/bundle/universalRelease/app-universal-release.aab`

## Install on a connected device

```bash
# Sign with a debug key
apksigner sign --ks debug.keystore --ks-key-alias androiddebugkey \
  --ks-pass pass:android --key-pass pass:android \
  --out fini-debug.apk app-universal-release-unsigned.apk

# Install
adb install fini-debug.apk

# Launch
adb shell am start -n com.fini.app/.MainActivity
```

## Prerequisites

- Android Studio (Flatpak: `com.google.AndroidStudio`)
- JDK (bundled with Android Studio)
- Android SDK + NDK 29+
- Rust Android targets:
  ```bash
  rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
  ```
- Environment variables in `~/.bashrc`:
  ```bash
  export JAVA_HOME="<path-to-jdk>"        # e.g. bundled JBR inside Android Studio
  export ANDROID_HOME="$HOME/Android/Sdk"
  export NDK_HOME="$ANDROID_HOME/ndk/<ndk-version>"
  PATH="$JAVA_HOME/bin:$ANDROID_HOME/platform-tools:$ANDROID_HOME/cmdline-tools/latest/bin:$PATH"
  ```
