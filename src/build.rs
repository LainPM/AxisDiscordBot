use anyhow::Result;
use vergen::EmitBuilder;

fn main() -> Result<()> {
    // Generate build-time environment variables
    EmitBuilder::builder()
        .build_timestamp()
        .rustc_semver()
        .cargo_target_triple()
        .emit()?;
    
    Ok(())
}
