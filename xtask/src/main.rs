//! This repository's `cargo xtask`: the shared tooling in `mxm-xtask` — `bundle`, which stages
//! the control map beside the bundle, and `fetch`, which builds other repositories' plugins this
//! repository's tests load.

fn main() -> nice_plug_xtask::Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits directly in the workspace root");
    mxm_xtask::main(root)
}
