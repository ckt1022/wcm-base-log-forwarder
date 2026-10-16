#!/usr/bin/env python3
"""由正式 wcm-b1 設定 (forwarder_membound_b1.yaml) 產生只差 plugin_call_size 的變體。

用法: make_yaml.py <base.yaml> <out.yaml> <k> [tcp_port] [sink_port]
  k = 0 代表 unlimited（不寫入 plugin_call_size 欄位，與原檔逐字相同）。
  tcp_port / sink_port 只供本機 smoke test 換埠用；正式量測不得指定。
"""
import sys

base, out, k = sys.argv[1], sys.argv[2], int(sys.argv[3])
tcp_port = sys.argv[4] if len(sys.argv) > 4 else None
sink_port = sys.argv[5] if len(sys.argv) > 5 else None

lines = open(base).read().splitlines(keepends=True)
res = []
inserted = False
for line in lines:
    if tcp_port and line.strip() == "port: 5140":
        line = line.replace("5140", tcp_port)
    if sink_port and "127.0.0.1:8080" in line:
        line = line.replace("127.0.0.1:8080", "127.0.0.1:" + sink_port)
    res.append(line)
    if line.strip() == "plugin_instance_reuse: true" and k > 0:
        indent = line[: len(line) - len(line.lstrip())]
        res.append(f"{indent}plugin_call_size: {k}\n")
        inserted = True
if k > 0 and not inserted:
    sys.exit("plugin_instance_reuse line not found; refusing to guess")
if k > 0:
    res[0] = res[0].rstrip("\n") + f" | per-record study: plugin_call_size={k}\n"
open(out, "w").write("".join(res))
print(f"wrote {out} (k={k})")
