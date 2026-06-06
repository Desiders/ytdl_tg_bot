fn main() {
    let protoc = protoc_bin_vendored::protoc_bin_path().unwrap();
    std::env::set_var("PROTOC", protoc);
    tonic_prost_build::compile_protos("proto/downloader.proto").unwrap();
}
