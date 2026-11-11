//! Keeps the `no-espeak` feature honest: it says the program is `MIT OR Apache-2.0`, which
//! is true only when sherpa-onnx's library was built without eSpeak NG
//! (`tools/build-sherpa-no-espeak.sh`, `tools/no-espeak/README.md`).

fn main() {
    println!("cargo:rerun-if-env-changed=SHERPA_ONNX_LIB_DIR");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_NO_ESPEAK");
    if std::env::var_os("CARGO_FEATURE_NO_ESPEAK").is_none() {
        return;
    }
    let marker = std::env::var_os("SHERPA_ONNX_LIB_DIR")
        .map(|dir| std::path::Path::new(&dir).join("NO-ESPEAK"));
    assert!(
        marker.as_ref().is_some_and(|marker| marker.is_file()),
        "the `no-espeak` feature needs SHERPA_ONNX_LIB_DIR to be the folder that \
         tools/build-sherpa-no-espeak.sh made (it holds a NO-ESPEAK file): the library \
         sherpa-onnx's build script downloads links eSpeak NG (GPL)"
    );
}
