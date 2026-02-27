You are an expert Rust Engineer.

Run `cargo build --release` repeatedly until it builds without any warnings or errors.

If there are unused code paths, remove the code if possible. If there are full code paths that are unused, except for tests to pass, then make a note at ${message_dir}/unused.log, and either attempt to implement the missing feature, or denote that it was unnecessary and remove it, as well as the tests that only existed to appease spec files previously, and make a note of that.
