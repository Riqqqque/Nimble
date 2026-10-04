//! Embeds the manifest, icon, and version resource. The version block is
//! generated from Cargo.toml so it can't drift; Windows' default-app picker
//! only lists programs whose exe carries a FileDescription.

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=nimble.manifest");
    println!("cargo:rerun-if-changed=nimble.ico");
    println!("cargo:rerun-if-changed=Cargo.toml");

    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let abs = |f: &str| Path::new(&dir).join(f).display().to_string().replace('\\', "\\\\");
    let ver = env!("CARGO_PKG_VERSION");
    let mut nums: Vec<u32> = ver.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    nums.resize(4, 0);
    let commas = nums.iter().map(u32::to_string).collect::<Vec<_>>().join(",");

    let mut rc = format!("1 24 \"{}\"\n", abs("nimble.manifest"));
    if Path::new(&dir).join("nimble.ico").exists() {
        rc += &format!("1 ICON \"{}\"\n", abs("nimble.ico"));
    }
    rc += &format!(
        r#"
1 VERSIONINFO
FILEVERSION {commas}
PRODUCTVERSION {commas}
FILEFLAGSMASK 0x3F
FILEFLAGS 0
FILEOS 0x40004
FILETYPE 1
FILESUBTYPE 0
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904B0"
        BEGIN
            VALUE "CompanyName", "Rique"
            VALUE "FileDescription", "Nimble"
            VALUE "FileVersion", "{ver}"
            VALUE "InternalName", "nimble"
            VALUE "OriginalFilename", "nimble.exe"
            VALUE "ProductName", "Nimble"
            VALUE "ProductVersion", "{ver}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#
    );
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("nimble.rc");
    std::fs::write(&out, rc).expect("write nimble.rc");
    embed_resource::compile(&out, embed_resource::NONE)
        .manifest_required()
        .expect("failed to embed resources");
}
