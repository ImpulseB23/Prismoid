fn main() {
    // Re-run when the YouTube OAuth credentials change so cached
    // builds don't bake in stale values from a previous environment.
    println!("cargo:rerun-if-env-changed=GOOGLE_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=GOOGLE_CLIENT_SECRET");
    tauri_build::build()
}
