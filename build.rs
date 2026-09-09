use std::{env, fs, path::PathBuf, process::Command};

fn resource_compiler() -> PathBuf {
    if let Some(path) = env::var_os("RC") {
        return path.into();
    }

    let sdk_root = env::var_os("WindowsSdkDir").map(PathBuf::from).or_else(|| {
        env::var_os("ProgramFiles(x86)")
            .map(PathBuf::from)
            .map(|path| path.join("Windows Kits/10"))
    });
    let host_arch = if env::var("HOST").is_ok_and(|host| host.starts_with("aarch64")) {
        "arm64"
    } else {
        "x64"
    };

    if let Some(bin_dir) = sdk_root.map(|path| path.join("bin"))
        && let Ok(entries) = fs::read_dir(bin_dir)
    {
        let mut versions = entries
            .flatten()
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        versions.sort_unstable();
        for version in versions.into_iter().rev() {
            let candidate = version.join(host_arch).join("rc.exe");
            if candidate.is_file() {
                return candidate;
            }
        }
    }

    PathBuf::from("rc.exe")
}

fn main() {
    println!("cargo:rerun-if-changed=packaging/codex-gui.png");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let icon_source = manifest_dir.join("packaging/codex-gui.png");
    let generated_icon = out_dir.join("codex-gui.ico");
    let resource_script = out_dir.join("codex-gui.rc");
    let resource = out_dir.join("codex-gui.res");

    let status = Command::new(env::var_os("MAGICK").unwrap_or_else(|| "magick".into()))
        .arg(icon_source)
        .args(["-define", "icon:auto-resize=256,128,64,48,32,16"])
        .arg(&generated_icon)
        .status()
        .expect("failed to run ImageMagick (magick) to generate the Windows icon");
    assert!(
        status.success(),
        "ImageMagick failed to generate the Windows icon"
    );

    fs::write(&resource_script, "1 ICON \"codex-gui.ico\"\n")
        .expect("failed to write the Windows resource script");
    let status = Command::new(resource_compiler())
        .current_dir(out_dir)
        .args([
            "/nologo",
            &format!("/fo{}", resource.display()),
            resource_script.file_name().unwrap().to_str().unwrap(),
        ])
        .status()
        .expect("failed to run the Windows resource compiler (rc.exe)");

    assert!(
        status.success(),
        "rc.exe failed to compile the application icon"
    );
    println!("cargo:rustc-link-arg-bin=codex-gui={}", resource.display());
}
