fn main() {
    // tauri_build's default Windows attributes embed the
    // Common-Controls v6 manifest as a .res resource on the bin target
    // only. Lib unit tests build a separate test exe with no manifest,
    // and on Windows the loader then resolves comctl32.dll to the
    // legacy v5 in System32 — which doesn't export TaskDialogIndirect
    // or the WindowSubclass APIs that wry/muda import. The test exe
    // fails to launch with STATUS_ENTRYPOINT_NOT_FOUND before any user
    // code runs.
    //
    // Cargo has no link-arg selector that covers lib unit tests
    // specifically (`-tests` is integration-tests-only), so opt out of
    // tauri_build's manifest and emit our own via `rustc-link-arg`,
    // which applies to every artifact: bin, integration tests, and the
    // lib unit-test exe. The content matches what tauri_build would
    // have shipped (just the v6 dependency declaration).
    let mut attributes = tauri_build::Attributes::new();
    #[cfg(target_os = "windows")]
    {
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
    }
    if let Err(e) = tauri_build::try_build(attributes) {
        eprintln!("tauri-build failed: {e:#}");
        std::process::exit(1);
    }

    #[cfg(target_os = "windows")]
    {
        const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
"#;
        let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
        let manifest_path = std::path::Path::new(&out_dir).join("kcc.manifest");
        std::fs::write(&manifest_path, MANIFEST).expect("write manifest");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg=/MANIFESTINPUT:{}",
            manifest_path.display()
        );
    }
}
