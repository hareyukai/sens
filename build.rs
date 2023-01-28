fn main() {
    println!("cargo:rustc-link-search=native={}", "/opt/homebrew/Cellar/sdl2/2.26.2/lib");
}
