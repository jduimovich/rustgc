
echo WARMUP Release Static
cargo run --release  > /dev/null
time cargo run --release 

echo WARMUP Release dynamic_mem
cargo run --release --features dynamic_mem  > /dev/null
time cargo run --release --features dynamic_mem
