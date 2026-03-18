fn main() {
    tonic_build::compile_protos("proto/downloader.proto").unwrap();
}
