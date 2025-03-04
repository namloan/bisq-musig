use std::prelude::rust_2021::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::compile_protos("src/main/proto/rpc.proto")?;
    Ok(())
}
