fn main() {
    let rc = if std::path::Path::new("nimble.ico").exists() {
        "nimble.rc"
    } else {
        "nimble.rc.manifest-only"
    };
    embed_resource::compile(rc, embed_resource::NONE)
        .manifest_required()
        .expect("failed to embed resources");
}
