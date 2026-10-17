// Compile la passerelle OCCT (cpp/occt_shim.cpp) et la lie au noyau OCCT.
// - Linux (natif)  : OCCT système (paquets apt 7.6.3).
// - Windows (cross): OCCT 7.6.3 cross-compilé mingw (dossier OCCT_WIN_DIR).
fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    // Bibliothèques OCCT requises (ordre sans importance pour le lien dynamique).
    let occt_libs = [
        "TKSTEP", "TKSTEP209", "TKSTEPAttr", "TKSTEPBase", "TKIGES", "TKXSBase", "TKSTL",
        "TKMesh", "TKPrim", "TKFillet", "TKBool", "TKBO", "TKTopAlgo", "TKGeomAlgo",
        "TKShHealing", "TKBRep", "TKGeomBase", "TKG3d", "TKG2d", "TKMath", "TKernel",
    ];

    let mut b = cc::Build::new();
    b.cpp(true).file("cpp/occt_shim.cpp").flag_if_supported("-std=c++17").warnings(false);

    if target_os == "windows" {
        // Dossier de l'install OCCT Windows (cross-build mingw). Surchargé par OCCT_WIN_DIR.
        let occt = std::env::var("OCCT_WIN_DIR")
            .unwrap_or_else(|_| "../winbuild/occt-win".to_string());
        b.include(format!("{occt}/inc"));
        b.define("_WIN32_WINNT", Some("0x0601"));
        b.define("_USE_MATH_DEFINES", None); // mingw : expose M_PI/M_PI_2 dans <cmath>
        b.compile("occt_shim");
        println!("cargo:rustc-link-search=native={occt}/win64/gcc/lib");
        // bibliothèques OCCT émises en fin de script (après la passerelle 3DM qui en dépend aussi)
    } else {
        b.include("/usr/include/opencascade").compile("occt_shim");
        println!("cargo:rustc-link-search=native=/usr/lib/x86_64-linux-gnu");
        for lib in occt_libs { println!("cargo:rustc-link-lib=dylib={lib}"); }
    }

    // Passerelle Assimp (3DS, DAE, FBX, glTF…) : Linux pour l'instant (cfg has_assimp)
    println!("cargo::rustc-check-cfg=cfg(has_assimp)");
    if target_os != "windows" && std::path::Path::new("/usr/include/assimp/Importer.hpp").exists() {
        cc::Build::new().cpp(true).file("cpp/assimp_shim.cpp").flag_if_supported("-std=c++17").warnings(false).compile("assimp_shim");
        println!("cargo:rustc-link-lib=dylib=assimp");
        println!("cargo:rustc-cfg=has_assimp");
    }
    // Windows : Assimp cross-compilé mingw (../winbuild/assimp-win, libassimp-5.dll à livrer)
    let aw = std::path::Path::new("../winbuild/assimp-win");
    if target_os == "windows" && aw.join("lib/libassimp.dll.a").exists() {
        cc::Build::new().cpp(true).file("cpp/assimp_shim.cpp").include(aw.join("include")).flag_if_supported("-std=c++17").warnings(false).compile("assimp_shim");
        println!("cargo:rustc-link-search=native={}", std::fs::canonicalize(aw.join("lib")).unwrap().display());
        println!("cargo:rustc-link-lib=dylib=assimp");
        println!("cargo:rustc-cfg=has_assimp");
    }
    println!("cargo:rerun-if-changed=cpp/assimp_shim.cpp");
    // Passerelle Rhino 3DM (openNURBS compilé dans ../libs/opennurbs) : cfg has_3dm
    println!("cargo::rustc-check-cfg=cfg(has_3dm)");
    let on = std::path::Path::new("../libs/opennurbs");
    if target_os != "windows" && on.join("build/libopennurbsStatic.a").exists() {
        cc::Build::new().cpp(true).file("cpp/on3dm_shim.cpp").include(on).include("/usr/include/opencascade").flag_if_supported("-std=c++17")
            .warnings(false).compile("on3dm_shim");
        let b = std::fs::canonicalize(on.join("build")).unwrap();
        println!("cargo:rustc-link-search=native={}", b.display());
        println!("cargo:rustc-link-search=native={}/freetype263", b.display());
        println!("cargo:rustc-link-search=native={}/android_uuid", b.display());
        println!("cargo:rustc-link-search=native={}/zlib", b.display());
        for l in ["opennurbsStatic", "opennurbs_public_freetype", "android_uuid", "zlib"] { println!("cargo:rustc-link-lib=static={l}"); }
        println!("cargo:rustc-cfg=has_3dm");
    }
    // Windows : openNURBS cross-compilé mingw (../winbuild/onurbs-w2, en-tête de compatibilité forcé)
    let onw = std::path::Path::new("../winbuild/onurbs-w2");
    if target_os == "windows" && onw.join("libopennurbsStatic.a").exists() {
        let occt = std::env::var("OCCT_WIN_DIR").unwrap_or_else(|_| "../winbuild/occt-win".to_string());
        let compat = std::fs::canonicalize(on.join("opennurbs_mingw_compat.h")).unwrap();
        cc::Build::new().cpp(true).file("cpp/on3dm_shim.cpp").include(on).include(format!("{occt}/inc"))
            .flag("-UWIN32").define("UNICODE", None).define("_UNICODE", None).define("_USE_MATH_DEFINES", None)
            .flag("-include").flag(compat.to_str().unwrap()).flag("-fpermissive").flag_if_supported("-std=c++17")
            .warnings(false).compile("on3dm_shim");
        let b = std::fs::canonicalize(onw).unwrap();
        // zlib d'openNURBS appelle zcalloc/zcfree définis DANS openNURBS : ces deux fonctions sont
        // isolées dans libon_zmem.a (ar rcs … opennurbs_zlib_memory.cpp.obj) et liées après zlib
        println!("cargo:rustc-link-search=native={}", b.display());
        println!("cargo:rustc-link-search=native={}/zlib", b.display());
        for l in ["opennurbsStatic", "zlib", "on_zmem"] { println!("cargo:rustc-link-lib=static={l}"); }
        for l in ["rpcrt4", "shlwapi", "ole32", "uuid", "shell32", "gdi32", "dwrite", "user32"] { println!("cargo:rustc-link-lib=dylib={l}"); }
        println!("cargo:rustc-cfg=has_3dm");
    }
    println!("cargo:rerun-if-changed=cpp/on3dm_shim.cpp");
    // Passerelle SketchUp (OpenSKP, licence MIT, compilé dans ../libs/skp-iamahsanmehmood) : cfg has_skp
    println!("cargo::rustc-check-cfg=cfg(has_skp)");
    let skp = std::path::Path::new("../libs/skp-iamahsanmehmood/packages/cpp");
    let skp_build = if target_os == "windows" { skp.join("build-win") } else { skp.join("build") };
    if skp_build.join("libOpenSkp.a").exists() {
        cc::Build::new().cpp(true).file("cpp/skp_shim.cpp").include(skp.join("include")).include(skp_build.join("include"))
            .include(skp_build.join("miniz_include")).define("OPENSKP_STATIC", None).flag_if_supported("-std=c++17").warnings(false).compile("skp_shim");
        let b = std::fs::canonicalize(&skp_build).unwrap();
        println!("cargo:rustc-link-search=native={}", b.display());
        println!("cargo:rustc-link-lib=static=OpenSkp");
        println!("cargo:rustc-cfg=has_skp");
    }
    println!("cargo:rerun-if-changed=cpp/skp_shim.cpp");
    // Windows : bibliothèque C++ en dernier (les .a statiques ci-dessus en dépendent)
    if target_os == "windows" {
        for lib in occt_libs { println!("cargo:rustc-link-lib=dylib={lib}"); }
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }
    println!("cargo:rerun-if-changed=cpp/occt_shim.cpp");
    println!("cargo:rerun-if-env-changed=OCCT_WIN_DIR");
}
