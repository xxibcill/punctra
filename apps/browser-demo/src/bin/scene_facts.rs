//! Prints the generated browser-demo scene facts as machine-readable JSON.

#![forbid(unsafe_code)]
#![allow(dead_code)]

#[path = "../diagnostics.rs"]
mod diagnostics;
#[path = "../display.rs"]
mod display;
#[path = "../host.rs"]
mod host;
#[path = "../scene.rs"]
mod scene;
#[path = "../streaming.rs"]
mod streaming;

fn main() {
    let scene = scene::PreparedScene::new().expect("deterministic generated scene must prepare");
    println!(
        "{}",
        serde_json::to_string(&scene.facts()).expect("scene facts must serialize")
    );
}
