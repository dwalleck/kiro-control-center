//! Smoke test that exists primarily to declare a `[[test]]` target so
//! `cargo:rustc-link-arg-tests` in `build.rs` is accepted. The manifest
//! injection that lives behind that directive is what lets the lib unit
//! tests launch on Windows; without it, `cargo test --lib` fails with
//! `STATUS_ENTRYPOINT_NOT_FOUND` because `comctl32.dll` resolves to the
//! legacy v5 in `System32`.

#[test]
fn binary_launches() {
    // If you can read this assertion firing, the manifest dance worked:
    // the test binary loaded all its DLL imports successfully.
}
