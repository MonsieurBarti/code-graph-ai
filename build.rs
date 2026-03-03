/// Build script for code-graph-cli.
///
/// When the `rag` feature is enabled, fastembed uses the `ort` crate which links against
/// a prebuilt ONNX Runtime binary. On systems with GLIBC < 2.38 or GCC < 13, the prebuilt
/// binary may reference symbols that don't exist in the system libraries:
///
/// - `__isoc23_strtol` / `__isoc23_strtoll` / `__isoc23_strtoull` (C23 strtol variants,
///   added in GLIBC 2.38) — we provide thin aliases backed by the classic strtol/strtoll
/// - `__cxa_call_terminate` (C++ exception handling helper emitted by GCC 13+ as an inline
///   helper in catch blocks) — we provide a stub that calls `std::terminate()`
///
/// The compat static library is compiled from `compat/ort_compat.cpp` and linked AFTER
/// all crate libraries via `cargo:rustc-link-arg` (which appends to the linker command end).
/// This ensures the weak stubs fill gaps without conflicting with real definitions.
///
/// This is only needed on aarch64 Linux with older glibc (GLIBC < 2.38) and older GCC (< 13).
/// x86_64 Linux and macOS are unaffected because they have newer toolchains or different ABIs.
#[cfg(feature = "rag")]
fn build_ort_compat() {
    use std::process::Command;

    // Compat shim only needed on Linux (glibc < 2.38 / GCC < 13).
    // macOS uses different ABI and doesn't need these symbol stubs.
    if !cfg!(target_os = "linux") {
        return;
    }

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR must be set by cargo");
    let src = std::path::Path::new("compat/ort_compat.cpp");

    if !src.exists() {
        return;
    }

    let obj_path = std::path::Path::new(&out_dir).join("ort_compat.o");
    let lib_path = std::path::Path::new(&out_dir).join("libort_compat.a");

    // Compile to object file — use g++ on Linux, c++ (clang++) on macOS
    let compiler = if cfg!(target_os = "linux") {
        "g++"
    } else {
        "c++"
    };
    let compile_status = Command::new(compiler)
        .args([
            "-c",
            "-fPIC",
            "-std=c++17",
            "-o",
            obj_path.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .status();

    match compile_status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!(
                "cargo:warning=ort_compat.cpp compilation failed (exit {}); \
                 build may fail with undefined reference errors on older glibc systems",
                s
            );
            return;
        }
        Err(e) => {
            eprintln!(
                "cargo:warning=g++ not found ({}); \
                 build may fail with undefined reference errors on older glibc systems",
                e
            );
            return;
        }
    }

    // Archive to static lib
    let ar_status = Command::new("ar")
        .args([
            "rcs",
            lib_path.to_str().unwrap(),
            obj_path.to_str().unwrap(),
        ])
        .status();

    if ar_status.map(|s| s.success()).unwrap_or(false) {
        // Use rustc-link-arg to append the compat lib AFTER all crate deps.
        // This is critical: the compat stubs must come AFTER the ORT static lib
        // so the linker resolves ORT's undefined symbols from the compat lib.
        // (cargo:rustc-link-lib would insert it BEFORE crate deps — wrong order.)
        //
        // --start-group/--end-group are GNU ld flags; macOS ld doesn't support them
        // (and doesn't need them — Darwin ld resolves symbols without grouping).
        if cfg!(target_os = "linux") {
            println!("cargo:rustc-link-arg=-Wl,--start-group");
            println!("cargo:rustc-link-arg={}", lib_path.display());
            println!("cargo:rustc-link-arg=-Wl,--end-group");
        } else {
            println!("cargo:rustc-link-search=native={}", out_dir);
            println!("cargo:rustc-link-lib=static=ort_compat");
        }
    } else {
        eprintln!("cargo:warning=failed to create ort_compat.a");
    }

    println!("cargo:rerun-if-changed=compat/ort_compat.cpp");
}

/// Ensure `web/dist/` exists when the `web` feature is active.
///
/// RustEmbed's `#[derive(RustEmbed)]` proc-macro expands at compile time and hard-errors
/// if the folder path does not exist on disk.  `web/dist/` is a generated build artifact
/// (gitignored) that is produced by running `npm run build` inside `web/`.  In CI — and
/// for any developer who hasn't built the frontend yet — the directory will be absent,
/// causing a confusing macro error instead of a clear "no assets" runtime behaviour.
///
/// This stub creates the directory (and a minimal `index.html` placeholder) when it is
/// missing, so that `cargo build --features web` always compiles.  At runtime the
/// placeholder serves a 404-like page; a proper build replaces the placeholder with real
/// assets via `npm run build`.
#[cfg(feature = "web")]
fn ensure_web_dist() {
    use std::fs;
    use std::path::Path;

    let dist = Path::new("web/dist");
    if !dist.exists() {
        fs::create_dir_all(dist)
            .expect("build.rs: failed to create web/dist/ placeholder directory");

        // Write a minimal index.html so RustEmbed has at least one file to embed.
        // This keeps the binary functional (it serves a human-readable message)
        // even when the frontend was never built.
        let placeholder = r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>code-graph</title></head>
<body>
<h1>Frontend not built</h1>
<p>Run <code>npm run build</code> inside the <code>web/</code> directory to generate the frontend assets.</p>
</body>
</html>
"#;
        fs::write(dist.join("index.html"), placeholder)
            .expect("build.rs: failed to write web/dist/index.html placeholder");

        println!(
            "cargo:warning=web/dist/ was missing — created a placeholder. \
             Run `cd web && npm run build` for a real frontend."
        );
    }

    // Re-run this script whenever web/dist changes so incremental builds stay correct.
    println!("cargo:rerun-if-changed=web/dist");
}

fn main() {
    #[cfg(feature = "web")]
    ensure_web_dist();

    #[cfg(feature = "rag")]
    build_ort_compat();
}
