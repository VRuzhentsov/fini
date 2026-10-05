# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.
#
# For more details, see
#   http://developer.android.com/guide/developing/tools/proguard.html

# If your project uses WebView with JS, uncomment the following
# and specify the fully qualified class name to the JavaScript interface
# class:
#-keepclassmembers class fqcn.of.javascript.interface.for.webview {
#   public *;
#}

# Uncomment this to preserve the line number information for
# debugging stack traces.
#-keepattributes SourceFile,LineNumberTable

# If you keep the line number information, uncomment this to
# hide the original source file name.
#-renamesourcefileattribute SourceFile

# ble-gatt's dev.blegatt bridge ships with tauri-plugin-ble-gatt, whose
# consumer rules keep it (Rust calls it by name over raw JNI).
# Reached only by name, from Rust, via `call_static_context_void` in
# `space_sync/commands.rs` -- never from Kotlin or Java, so R8 sees no
# reference and is free to rename or remove it. Manifest registration keeps
# the Android component alive but says nothing about this companion method,
# so without this rule a minified release APK starts and then dies on
# NoSuchMethodError the first time background sync tries to start (ADR-0004).
# Debug builds are unminified, which is exactly why this cannot be caught by
# the way the app is normally tested.
-keep class com.fini.app.SyncForegroundService { *; }
-keep class com.fini.app.SyncForegroundService$Companion { *; }
