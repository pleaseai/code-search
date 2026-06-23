// napi-rs codegen + linker setup. Configures the platform-specific link flags
// (e.g. `-undefined dynamic_lookup` on macOS) so the cdylib resolves Node-API
// symbols at load time, and emits the binding registration glue.
fn main() {
    napi_build::setup();
}
