#[cfg(target_family = "windows")]
fn main() {
    windows_exe_info::icon::icon_ico("docs/favicon.ico");
    windows_exe_info::versioninfo::link_cargo_env();
}

#[cfg(target_family = "unix")]
fn main() {}
