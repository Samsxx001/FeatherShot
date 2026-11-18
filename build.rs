use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=FEATHERSHOT_MINGW_BIN");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    let icon = out.join("feathershot.ico");
    let manifest = out.join("app.manifest");
    // A tiny code-drawn camera icon. No image framework or build dependency.
    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0, 1, 0, 32, 32, 0, 0, 1, 0, 32, 0]);
    ico.extend_from_slice(&(40u32 + 32 * 32 * 4 + 128).to_le_bytes());
    ico.extend_from_slice(&22u32.to_le_bytes());
    for n in [40u32, 32, 64] {
        ico.extend_from_slice(&n.to_le_bytes());
    }
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.extend_from_slice(&32u16.to_le_bytes());
    for n in [0u32, 32 * 32 * 4, 0, 0, 0, 0] {
        ico.extend_from_slice(&n.to_le_bytes());
    }
    for y in (0..32i32).rev() {
        for x in 0..32i32 {
            let camera = (4..28).contains(&x) && (8..26).contains(&y)
                || (10..20).contains(&x) && (5..8).contains(&y);
            let color = if camera {
                if (x - 16).pow(2) + (y - 17).pow(2) <= 28 {
                    [25, 19, 15, 255]
                } else {
                    [185, 232, 122, 255]
                }
            } else {
                [0, 0, 0, 0]
            };
            ico.extend_from_slice(&color);
        }
    }
    ico.extend_from_slice(&[0; 128]);
    fs::write(&icon, ico).expect("Write icon");
    fs::write(&manifest, r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3"><security><requestedPrivileges><requestedExecutionLevel level="asInvoker" uiAccess="false"/></requestedPrivileges></security></trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1"><application><supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/></application></compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3"><windowsSettings>
    <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
    <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
  </windowsSettings></application>
</assembly>"#).expect("Write manifest");
    let rc = out.join("app.rc");
    fs::write(
        &rc,
        format!(
            r#"
1 ICON "{}"
1 24 "{}"
1 VERSIONINFO
FILEVERSION 0,1,5,0
PRODUCTVERSION 0,1,5,0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
 BLOCK "StringFileInfo" BEGIN
  BLOCK "040904B0" BEGIN
   VALUE "FileDescription", "FeatherShot - Native Rust Screenshot Tool\0"
   VALUE "FileVersion", "0.1.5\0"
   VALUE "ProductName", "FeatherShot\0"
   VALUE "ProductVersion", "0.1.5\0"
  END
 END
 BLOCK "VarFileInfo" BEGIN
  VALUE "Translation", 0x409, 1200
 END
END
"#,
            icon.to_string_lossy().replace('\\', "/"),
            manifest.to_string_lossy().replace('\\', "/")
        ),
    )
    .expect("Write resource");
    let gnu = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default() == "gnu";
    let object = out.join(if gnu { "app.o" } else { "app.res" });
    let result = if gnu {
        Command::new("windres")
            .arg(&rc)
            .arg(&object)
            .args(["--input-format=rc", "--output-format=coff"])
            .status()
    } else {
        Command::new("rc")
            .arg("/nologo")
            .arg(format!("/fo{}", object.display()))
            .arg(&rc)
            .status()
    };
    match result {
        Ok(status) if status.success() => println!("cargo:rustc-link-arg={}", object.display()),
        Ok(_) => panic!("Windows resource compiler failed"),
        Err(e) => println!("cargo:warning=Resource compiler unavailable ({e}). The app works without an embedded icon/manifest."),
    }
}
