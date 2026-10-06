fn main() {
    println!("cargo:rerun-if-env-changed=GIXPYTHON_BUILD_GIX_REVISION");
    let revision = std::env::var("GIXPYTHON_BUILD_GIX_REVISION")
        .unwrap_or_else(|_| "f819565c2c4c56619c4888acef6cf3b8144cbccb".into());
    assert!(
        revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "GIXPYTHON_BUILD_GIX_REVISION must be a 40-character Git revision"
    );
    println!("cargo:rustc-env=GIXPYTHON_GIX_REVISION={revision}");
    pyo3_build_config::add_extension_module_link_args();
}
