fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // Restoring compiler-rt availability helpers dropped by Rust LTO (rust-lang/rust#164915)
        println!("cargo:rustc-link-arg=-fapple-link-rtlib");
    }
}
