use std::env;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=assets/brand/likoma.ico");
    if env::var("CARGO_CFG_TARGET_OS")? != "windows" {
        return Ok(());
    }
    winresource::WindowsResource::new()
        .set_icon("assets/brand/likoma.ico")
        .compile()
        .map_err(|error| {
            format!("failed to compile Windows icon resource from assets/brand/likoma.ico: {error}")
        })?;
    Ok(())
}
