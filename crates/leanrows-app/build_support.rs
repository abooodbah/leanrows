#[cfg(all(target_os = "windows", target_env = "msvc"))]
use std::path::{Path, PathBuf};

pub fn run() -> Result<(), String> {
    println!("cargo:rerun-if-changed=resources/leanrows.manifest");
    println!("cargo:rerun-if-changed=resources/leanrows.rc");
    println!("cargo:rerun-if-changed=../../branding/leanrows.ico");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION_MAJOR");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION_MINOR");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION_PATCH");

    #[cfg(all(target_os = "windows", target_env = "msvc"))]
    compile_windows_resources()?;

    Ok(())
}

#[cfg(all(target_os = "windows", target_env = "msvc"))]
fn compile_windows_resources() -> Result<(), String> {
    let manifest_dir = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR")
            .ok_or_else(|| String::from("CARGO_MANIFEST_DIR is unavailable"))?,
    );
    let output_dir = PathBuf::from(
        std::env::var_os("OUT_DIR").ok_or_else(|| String::from("OUT_DIR is unavailable"))?,
    );
    let resource_dir = manifest_dir.join("resources");
    let output = output_dir.join("leanrows.res");
    let generated_resource = output_dir.join("leanrows-version.rc");
    std::fs::write(&generated_resource, windows_version_resource()?)
        .map_err(|error| format!("could not write generated VERSIONINFO resource: {error}"))?;
    let compiler =
        find_resource_compiler().ok_or_else(|| String::from("Windows SDK rc.exe was not found"))?;
    let output_argument = format!("/fo{}", output.display());
    let include_argument = format!("/I{}", output_dir.display());
    let status = std::process::Command::new(compiler)
        .current_dir(&resource_dir)
        .env_remove("INCLUDE")
        .args([
            "/nologo",
            &output_argument,
            &include_argument,
            "leanrows.rc",
        ])
        .status()
        .map_err(|error| format!("could not start rc.exe: {error}"))?;
    if !status.success() {
        return Err(format!("rc.exe exited with {status}"));
    }
    println!("cargo:rustc-link-arg-bin=leanrows={}", output.display());
    Ok(())
}

#[cfg(all(target_os = "windows", target_env = "msvc"))]
fn windows_version_resource() -> Result<String, String> {
    let major = version_component("CARGO_PKG_VERSION_MAJOR")?;
    let minor = version_component("CARGO_PKG_VERSION_MINOR")?;
    let patch = version_component("CARGO_PKG_VERSION_PATCH")?;
    let dotted = format!("{major}.{minor}.{patch}.0");
    Ok(format!(
        r#"1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEFLAGSMASK 0x3FL
FILEFLAGS 0x0L
FILEOS 0x00040004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904B0"
        BEGIN
            VALUE "CompanyName", "Abdulfatah Bahbouh\0"
            VALUE "FileDescription", "LeanRows large row-file viewer\0"
            VALUE "FileVersion", "{dotted}\0"
            VALUE "InternalName", "leanrows\0"
            VALUE "LegalCopyright", "Copyright (c) 2026 Abdulfatah Bahbouh\0"
            VALUE "OriginalFilename", "leanrows.exe\0"
            VALUE "ProductName", "LeanRows\0"
            VALUE "ProductVersion", "{dotted}\0"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x0409, 1200
    END
END
"#
    ))
}

#[cfg(all(target_os = "windows", target_env = "msvc"))]
fn version_component(name: &str) -> Result<u16, String> {
    let value = std::env::var(name).map_err(|_| format!("{name} is unavailable"))?;
    value
        .parse::<u16>()
        .map_err(|error| format!("{name} is not a Windows version component: {error}"))
}

#[cfg(all(target_os = "windows", target_env = "msvc"))]
fn find_resource_compiler() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("PATH").and_then(find_on_path) {
        return Some(path);
    }
    let program_files = std::env::var_os("ProgramFiles(x86)")?;
    let bin = PathBuf::from(program_files).join("Windows Kits/10/bin");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(bin)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("x64/rc.exe"))
        .filter(|path| path.is_file())
        .collect();
    versions.sort_unstable();
    versions.pop()
}

#[cfg(all(target_os = "windows", target_env = "msvc"))]
fn find_on_path(path: std::ffi::OsString) -> Option<PathBuf> {
    let result = std::env::split_paths(&path)
        .map(|directory| directory.join(Path::new("rc.exe")))
        .find(|candidate| candidate.is_file());
    drop(path);
    result
}
