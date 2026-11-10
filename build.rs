fn main() {
    println!(
        "cargo:rustc-env=MDCONV_TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
