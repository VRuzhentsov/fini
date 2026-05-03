use std::env;

fn main() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

    let lib_dir = if target_os == "android" {
        let abi = match target_arch.as_str() {
            "aarch64" => "arm64-v8a",
            "arm" => "armeabi-v7a",
            "x86_64" => "x86_64",
            _ => "x86",
        };
        let abi_key = format!(
            "SHERPA_ONNX_LIB_DIR_ANDROID_{}",
            abi.replace('-', "_").to_uppercase()
        );
        env::var(&abi_key)
            .or_else(|_| {
                env::var("SHERPA_ONNX_LIB_DIR_ANDROID")
                    .map(|p| format!("{}/{}", p.trim_end_matches('/'), abi))
            })
            .or_else(|_| env::var("SHERPA_ONNX_LIB_DIR"))
            .ok()
    } else {
        env::var("SHERPA_ONNX_LIB_DIR").ok()
    };

    match &lib_dir {
        Some(path) => {
            println!("cargo:rustc-link-search=native={}", path);
            if target_os == "linux" || target_os == "macos" {
                println!("cargo:rustc-link-arg=-Wl,-rpath,{}", path);
            }
        }
        None => {
            println!(
                "cargo:warning=No sherpa-onnx lib dir found. \
                Set SHERPA_ONNX_LIB_DIR (desktop) or SHERPA_ONNX_LIB_DIR_ANDROID (android)."
            );
        }
    }

    println!("cargo:rustc-link-lib=dylib=sherpa-onnx-c-api");
    println!("cargo:rustc-link-lib=dylib=onnxruntime");

    println!("cargo:rerun-if-env-changed=SHERPA_ONNX_LIB_DIR");
    println!("cargo:rerun-if-env-changed=SHERPA_ONNX_LIB_DIR_ANDROID");
}
