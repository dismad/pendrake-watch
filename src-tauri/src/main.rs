// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "linux")]
    {
        // Avoid WebKitGTK 2.52.x exit-time GL/EGL teardown race
        // https://bugs.webkit.org/show_bug.cgi?id=303336
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
        // Optional second workaround if issues remain on some GPUs:
        // std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    pendrake_watch_lib::run()
}
