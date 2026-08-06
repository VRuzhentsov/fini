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

# ble-gatt's dev.blegatt bridge and Fini's own com.fini.app.BluetoothPairing
# are called by fixed class/method name and signature directly from Rust via
# raw JNI (services::android_context, ble_gatt::backend::android) -- not
# through Tauri's plugin/reflection machinery, which keeps its own generated
# classes automatically. Without these rules R8 strips or renames them in
# release builds (isMinifyEnabled = true here), and every JNI call from Rust
# starts throwing ClassNotFoundException/NoSuchMethodError while debug
# builds keep working fine -- a gap that only shows up in a release package.
-keep class dev.blegatt.BleGattBridge { *; }
-keep class dev.blegatt.NativeKt { *; }
-keepclasseswithmembernames class dev.blegatt.NativeKt { native <methods>; }
-keep class com.fini.app.BluetoothPairing { *; }