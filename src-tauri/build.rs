#[path = "apple-speech/build.rs"]
mod apple_speech_build;
#[path = "apple-translation/build.rs"]
mod apple_translation_build;
#[path = "src/core/development_build_permissions.rs"]
mod development_build_permissions;

#[path = "local-speech/build.rs"]
mod local_speech_build;

fn main() {
    local_speech_build::build();
    apple_speech_build::build();
    apple_translation_build::build();
    println!("cargo:rerun-if-changed=permissions/app.toml");
    let template = std::fs::read_to_string("permissions/app.toml")
        .expect("application permissions must be readable");
    let permissions = development_build_permissions::permissions_for_build(
        &template,
        std::env::var_os("CARGO_FEATURE_DEVELOPMENT_DEBUGGER").is_some(),
    );
    let output =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo output directory"))
            .join("app-permissions.toml");
    if std::fs::read_to_string(&output).ok().as_deref() != Some(permissions.as_str()) {
        std::fs::write(&output, permissions).expect("write build-specific application permissions");
    }
    let pattern = development_build_permissions::escape_glob_path(
        &output.to_string_lossy().replace('\\', "/"),
    );
    let manifest = tauri_build::AppManifest::new()
        .permissions_path_pattern(Box::leak(pattern.into_boxed_str()));
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
        .expect("Tauri build configuration");
}
