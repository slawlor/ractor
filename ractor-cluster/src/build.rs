// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! This is the pre-compilation build script for the crate `ractor` when running in distrubted
//! mode. It's used to compile protobuf into Rust code prior to compilation.

/// The shared-path for all protobuf specifications
const PROTOBUF_BASE_DIRECTORY: &str = "src/protocol";
/// The list of protobuf files to generate inside PROBUF_BASE_DIRECTORY
const PROTOBUF_FILES: [&str; 4] = ["meta", "node", "auth", "control"];

fn build_protobufs() {
    std::env::set_var("PROTOC", protobuf_src::protoc());

    let mut protobuf_files = Vec::with_capacity(PROTOBUF_FILES.len());

    for file in PROTOBUF_FILES.iter() {
        let proto_file = format!("{PROTOBUF_BASE_DIRECTORY}/{file}.proto");
        println!("cargo:rerun-if-changed={proto_file}");
        protobuf_files.push(proto_file);
    }

    prost_build::compile_protos(&protobuf_files, &[PROTOBUF_BASE_DIRECTORY]).unwrap();
}

fn main() {
    // compile the spec files into Rust code
    build_protobufs();
}
