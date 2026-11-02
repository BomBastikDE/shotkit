use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn add_if_file(candidates: &mut Vec<PathBuf>, path: PathBuf) {
    if path.is_file() && !candidates.contains(&path) {
        candidates.push(path);
    }
}

fn msvc_resource_compilers() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for variable in ["RC", "WindowsSdkDir", "VCToolsInstallDir"] {
        if let Some(value) = env::var_os(variable) {
            let path = PathBuf::from(value);
            if variable == "RC" {
                add_if_file(&mut candidates, path);
            } else if variable == "WindowsSdkDir" {
                let bin = path.join("bin");
                if let Ok(versions) = fs::read_dir(&bin) {
                    let mut versions: Vec<_> = versions.flatten().map(|entry| entry.path()).collect();
                    versions.sort();
                    for version in versions.into_iter().rev() {
                        for arch in ["x64", "x86"] {
                            add_if_file(&mut candidates, version.join(arch).join("rc.exe"));
                        }
                    }
                }
            } else {
                for arch in ["Hostx64/x64", "Hostx86/x86"] {
                    add_if_file(&mut candidates, path.join("bin").join(arch).join("rc.exe"));
                }
            }
        }
    }

    if let Some(program_files) = env::var_os("ProgramFiles(x86)") {
        let bin = PathBuf::from(program_files).join("Windows Kits/10/bin");
        if let Ok(versions) = fs::read_dir(bin) {
            let mut versions: Vec<_> = versions.flatten().map(|entry| entry.path()).collect();
            versions.sort();
            for version in versions.into_iter().rev() {
                for arch in ["x64", "x86"] {
                    add_if_file(&mut candidates, version.join(arch).join("rc.exe"));
                }
            }
        }
    }
    candidates
}

fn run_compiler(program: &Path, args: &[OsString], resource_dir: &Path) -> Result<(), String> {
    match Command::new(program)
        .args(args)
        .current_dir(resource_dir)
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("{} exited with {status}", program.display())),
        Err(error) => Err(format!("could not run {}: {error}", program.display())),
    }
}

fn main() {
    println!("cargo:rerun-if-changed=resources/shotkit.rc");
    println!("cargo:rerun-if-changed=resources/shotkit.ico");
    for variable in ["RC", "WindowsSdkDir", "WindowsSDKVersion", "VCToolsInstallDir"] {
        println!("cargo:rerun-if-env-changed={variable}");
    }

    let target = env::var("TARGET").expect("Cargo did not provide TARGET");
    if !target.contains("windows") {
        return;
    }

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo did not provide CARGO_MANIFEST_DIR"));
    let resource_dir = manifest_dir
        .join("resources")
        .canonicalize()
        .expect("could not resolve the ShotKit resource directory");
    let rc_file = resource_dir.join("shotkit.rc");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo did not provide OUT_DIR"));
    let output = out_dir
        .canonicalize()
        .expect("could not resolve Cargo OUT_DIR")
        .join("shotkit.res");

    let mut errors = Vec::new();
    if target.contains("msvc") {
        let mut compilers = msvc_resource_compilers();
        compilers.extend([PathBuf::from("rc.exe"), PathBuf::from("llvm-rc")]);
        for compiler in compilers {
            let args = if compiler.file_name().is_some_and(|name| name == "llvm-rc") {
                vec![OsString::from("/nologo"), OsString::from("/FO"), output.as_os_str().to_owned(), rc_file.as_os_str().to_owned()]
            } else {
                vec![OsString::from("/nologo"), OsString::from(format!("/fo{}", output.display())), rc_file.as_os_str().to_owned()]
            };
            match run_compiler(&compiler, &args, &resource_dir) {
                Ok(()) => {
                    println!("cargo:rustc-link-arg={}", output.display());
                    return;
                }
                Err(error) => errors.push(error),
            }
        }
    } else {
        let arch = target.split('-').next().unwrap_or("x86_64");
        let mut compilers = Vec::new();
        if let Some(rc) = env::var_os("RC") {
            compilers.push(PathBuf::from(rc));
        }
        compilers.extend([
            PathBuf::from(format!("{arch}-w64-mingw32-windres")),
            PathBuf::from("windres"),
        ]);
        for compiler in compilers {
            let args = [
                OsString::from("-O"),
                OsString::from("coff"),
                rc_file.as_os_str().to_owned(),
                output.as_os_str().to_owned(),
            ];
            match run_compiler(&compiler, &args, &resource_dir) {
                Ok(()) => {
                    println!("cargo:rustc-link-arg={}", output.display());
                    return;
                }
                Err(error) => errors.push(error),
            }
        }
    }

    panic!("failed to compile the ShotKit Windows icon resource; tried: {}. Set RC to a compatible resource compiler", errors.join("; "));
}
