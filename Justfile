build:
  @cargo build --workspace

run path: build
  @./target/debug/flang-design "./examples/{{path}}"
  @"./examples/build/{{file_stem(path)}}"; status=$?; echo "program exited with $status"