use std::path::PathBuf;
use std::{env, fs};

fn main() {
    println!("cargo:rerun-if-changed=shaders");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    let compiler = shaderc::Compiler::new().expect("failed to initialize shaderc");

    for entry in fs::read_dir("shaders").expect("shaders directory missing") {
        let path = entry.expect("unreadable shaders directory entry").path();
        let kind = match path.extension().and_then(|e| e.to_str()) {
            Some("vert") => shaderc::ShaderKind::Vertex,
            Some("frag") => shaderc::ShaderKind::Fragment,
            Some("comp") => shaderc::ShaderKind::Compute,
            _ => continue,
        };
        println!("cargo:rerun-if-changed={}", path.display());

        let file_name = path.file_name().unwrap().to_str().unwrap();
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        let artifact = compiler
            .compile_into_spirv(&source, kind, file_name, "main", None)
            .unwrap_or_else(|e| panic!("{e}"));

        fs::write(out_dir.join(format!("{file_name}.spv")), artifact.as_binary_u8())
            .unwrap_or_else(|e| panic!("failed to write {file_name}.spv: {e}"));
    }
}
