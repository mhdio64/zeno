# The crate is published as `zeno-cli`, the binary as `zeno`

The product's name is zeno and the executable it installs is `zeno`, but the crate published to crates.io is called `zeno-cli`, because `zeno` is already an unrelated Rust rasterization library with millions of downloads. Cargo package names and binary names are independent, so `cargo install zeno-cli` gives the user a `zeno` command. This is a compromise forced by the registry rather than a preference, and it should be revisited if the brand ever changes.
