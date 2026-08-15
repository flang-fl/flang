run path:
  @cargo run -- "./examples/{{path}}"
  @"./examples/build/{{file_stem(path)}}"; status=$?; echo "program exited with $status"