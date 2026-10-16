#!/bin/bash
# 收集重現所需的環境資訊（OS、kernel、Docker、image digest、工具鏈、plugin 雜湊）。
# 用法: collect_env.sh > environment.txt
set -u
MAIN=/home/ckt1022/wcm-base-log-forwarder
echo "# collected $(date -Is)"
echo "## OS / kernel"
uname -a
cat /etc/os-release 2>/dev/null | grep -E '^(PRETTY_NAME|VERSION_ID)='
echo "nproc=$(nproc)"
free -m | head -2
echo "clocksource=$(cat /sys/devices/system/clocksource/clocksource0/current_clocksource 2>/dev/null)"
echo "## Docker"
docker version --format 'client={{.Client.Version}} server={{.Server.Version}}' 2>&1
for t in wcm-base-log-forwarder:percall wcm-base-log-forwarder:memguard rust:1.93-slim-bookworm debian:bookworm-slim; do
    echo "image $t id=$(docker image inspect --format '{{.Id}}' "$t" 2>/dev/null) repodigests=$(docker image inspect --format '{{json .RepoDigests}}' "$t" 2>/dev/null)"
done
echo "## Toolchain"
rustc --version; cargo --version
rustup target list --installed 2>/dev/null | tr '\n' ' '; echo
wasm-tools --version 2>&1
python3 --version; python3 -c 'import numpy; print("numpy", numpy.__version__)' 2>&1
node --version 2>&1 | sed 's/^/node /'
go version 2>&1
echo "## Git"
git -C "$(dirname "$0")/../../../.." log -1 --format='worktree HEAD %H %s' 2>&1
git -C "$(dirname "$0")/../../../.." branch --show-current 2>&1
echo "## Plugin / harness binaries (sha256)"
sha256sum "$MAIN"/test/ours/latency-benchmark-Rust/plugin/*.wasm \
    "$MAIN"/test-plugins/rust-plugin/{parse,filter,format}/target/wasm32-wasip2/release/{parse,filter,format}.wasm \
    "$MAIN"/test-plugins/c-plugin/parse/parser_c_json.wasm "$MAIN"/test-plugins/c-plugin/filter/latency_c_filter.wasm \
    "$MAIN"/test-plugins/c-plugin/format/format_json-flat.wasm \
    "$MAIN"/test-plugins/csharp-plugin/parse/parser_csharp.wasm "$MAIN"/test-plugins/csharp-plugin/filter/filter_csharp.wasm \
    "$MAIN"/test-plugins/go-plugin/parse/parser_json.wasm \
    "$MAIN"/test/compare/capacity/{gen_bin,httpsink_bin,eval_trial.py,calc_latency_stream.py} \
    "$MAIN"/test/compare/correctness/{correctgen_bin,validationsink_bin,evaluate.py} \
    "$MAIN"/test/ours/latency-benchmark-Rust/forwarder_membound_b1.yaml 2>/dev/null
