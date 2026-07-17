echo "Building image-mcp..."
cargo build --release
echo "Copying image-mcp to ~/.local/bin/..."
cp target/release/image-mcp ~/.local/bin/