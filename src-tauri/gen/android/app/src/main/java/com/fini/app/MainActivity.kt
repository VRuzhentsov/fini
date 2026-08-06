package com.fini.app

import android.os.Bundle
import androidx.activity.enableEdgeToEdge
import app.tauri.plugin.PluginManager

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    // PluginManager.onActivityCreate registers ActivityResultLaunchers (required before onStart).
    // Tauri's Rust bootstrap never calls this, so we initialize it here.
    PluginManager.onActivityCreate(this)
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // Bluetooth runtime permissions are deliberately NOT requested here.
    // See BluetoothPairing.requestPermissionsIfNeeded's doc comment: the
    // prompt is only triggered from a genuine user action (the Bluetooth
    // toggle in Device settings), not on every app launch.
  }
}
