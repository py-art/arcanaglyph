// crates/arcanaglyph-app/src/build.rs

fn main() {
    // Под Linux добавляем RPATH = /usr/lib/arcanaglyph/ в ELF-бинарь. После установки
    // .deb-пакета dynamic loader будет искать там libvosk.so и libonnxruntime.so
    // (которые мы кладём в /usr/lib/arcanaglyph/ внутри пакета). Системный ld.so
    // ищет /usr/local/lib раньше RPATH (через ld.so.conf), поэтому self-build
    // пользователя в /usr/local/lib/{libvosk,libonnxruntime}.so остаётся приоритетнее
    // bundled — что и требуется (см. setup_ort_dylib_path() в main.rs).
    //
    // Для dev-сборок (`make run` / `cargo run`) RPATH тоже добавляется, но не мешает:
    // dynamic loader не найдёт /usr/lib/arcanaglyph/ (директории нет) и пойдёт дальше
    // по LD_LIBRARY_PATH (Makefile ставит /usr/local/lib).
    #[cfg(target_os = "linux")]
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/arcanaglyph");

    tauri_build::build()
}
