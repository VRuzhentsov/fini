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
  }
}
