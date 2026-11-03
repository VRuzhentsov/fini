fn main() {
    // `embed_migrations!` (services/db.rs) reads this folder when db.rs
    // compiles; without this line an incremental build keeps the old set,
    // and a new migration does not run (diesel_migrations documentation).
    println!("cargo:rerun-if-changed=migrations");
    #[cfg(feature = "ui-plane")]
    tauri_build::build();
}
