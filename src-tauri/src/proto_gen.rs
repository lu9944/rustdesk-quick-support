// Generated RustDesk protobuf code.
// `mod.rs` declares `pub mod message;` / `pub mod rendezvous;`, which Rust
// resolves relative to the included mod.rs location (OUT_DIR/protos/). Loading
// them as module files (not via `include!`) is what makes the generated files'
// inner `#![...]` attributes valid.
pub mod protos {
    include!(concat!(env!("OUT_DIR"), "/protos/mod.rs"));
}

pub use protos::message;
pub use protos::rendezvous;
