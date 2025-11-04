fn main() {
    println!("cargo:rerun-if-changed=src/spfresh_wrapper.cpp");
    println!("cargo:rerun-if-changed=SPFresh/");

    let sptag_path = std::path::Path::new("SPFresh/SPFresh");
    
    // Forcing path to 'Release' as confirmed by user.
    let lib_path = sptag_path.join("Release");
    
    // Debug: print what files exist in the lib directory
    println!("cargo:warning=Looking for SPFresh libs in: {}", lib_path.display());
    if lib_path.exists() {
        if let Ok(entries) = std::fs::read_dir(&lib_path) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if name.contains("SPTAG") || name.contains("Distance") || name.ends_with(".so") || name.ends_with(".a") {
                        println!("cargo:warning=Found lib file: {}", name);
                    }
                }
            }
        }
    } else {
        println!("cargo:warning=Library path confirmed by user does not exist: {}", lib_path.display());
    }
    
    // Link pre-built SPFresh libraries
    println!("cargo:rustc-link-search=native={}", lib_path.display());
    
    // Try linking shared library first, fall back to static if needed
    println!("cargo:rustc-link-lib=dylib=SPTAGLib");
    
    // DistanceUtils might not exist in all builds - try to link if available
    if lib_path.join("libDistanceUtils.so").exists() {
        println!("cargo:rustc-link-lib=dylib=DistanceUtils");
    } else if lib_path.join("libDistanceUtils.a").exists() {
        println!("cargo:rustc-link-lib=static=DistanceUtils");
    } else {
        println!("cargo:warning=DistanceUtils library not found - SPFresh might not need it or it's embedded in SPTAGLib");
    }
    
    // Link system libraries - order matters for some linkers
    // Link stdc++ dynamically
    println!("cargo:rustc-link-lib=dylib=stdc++");
    println!("cargo:rustc-link-lib=dylib=gcc_s");    // GCC runtime support (for exceptions, etc.)
    println!("cargo:rustc-link-lib=dylib=gomp");     // OpenMP
    println!("cargo:rustc-link-lib=dylib=pthread");
    println!("cargo:rustc-link-lib=dylib=m");        // Math library
    println!("cargo:rustc-link-lib=dylib=dl");       // Dynamic loading
    
    // Compile our C++ wrapper
    cc::Build::new()
        .cpp(true)
        .file("src/spfresh_wrapper.cpp")
        .include(sptag_path)
        .include(sptag_path.join("AnnService"))
        .include(sptag_path.join("AnnService/inc"))
        .flag("-std=c++14")
        .flag("-O3")
        .flag("-fopenmp")
        .cpp_link_stdlib("stdc++")  // Explicitly link C++ stdlib
        .warnings(false)
        .compile("spfresh_wrapper");
}
