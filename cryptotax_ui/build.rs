fn main() {
    let config =
        slint_build::CompilerConfiguration::new()
        .with_style("fluent-dark".into());
    slint_build::compile_with_config("ui/appwindow.slint", config).unwrap();
}
