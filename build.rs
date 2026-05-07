fn main() {
    // Embed rscapt.ico into the exe when targeting Windows.
    // embed-resource is a no-op on non-Windows targets, so this is safe for
    // local Linux builds too.
    embed_resource::compile("rscapt.rc", embed_resource::NONE);
}
