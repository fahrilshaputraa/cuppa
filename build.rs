// Recompile when a migration is added or changed: `sqlx::migrate!` embeds the directory at compile time.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
