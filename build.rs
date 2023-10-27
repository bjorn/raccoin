#[cfg(target_family = "windows")]
fn main() {
    use std::path::Path;

    windows_exe_info::icon::icon_ico(Path::new("docs/favicon.ico"));
}

#[cfg(target_family = "unix")]
fn main() {}
