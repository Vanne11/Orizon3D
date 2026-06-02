use std::path::PathBuf;

fn main() {
    // Carpeta donde vive la librería oficial vendorizada (lib3DCamera.so).
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let lib_dir = manifest_dir.join("vendor/3DCamera/lib");

    if !lib_dir.join("lib3DCamera.so").exists() {
        panic!(
            "No se encontró lib3DCamera.so en {}. \
             Descárgala primero con:  ./scripts/fetch-sdk.sh",
            lib_dir.display()
        );
    }

    // Buscar y enlazar contra lib3DCamera.so en tiempo de compilación.
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=dylib=3DCamera");

    // Embebemos un rpath absoluto para que el binario encuentre la .so en
    // tiempo de ejecución sin necesidad de exportar LD_LIBRARY_PATH.
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());

    // Recompilar si cambia la librería.
    println!(
        "cargo:rerun-if-changed={}",
        lib_dir.join("lib3DCamera.so").display()
    );
}
