cp ./target/release/decree ~/.cargo/bin/decree.new
cp ~/.cargo/bin/decree decree.backup
mv ~/.cargo/bin/decree.new ~/.cargo/bin/decree
