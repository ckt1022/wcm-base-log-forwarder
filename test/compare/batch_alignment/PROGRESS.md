# batch_alignment 進度紀錄（方法 1、2、3）

## 已完成（2026-10-14 03:3x）
- 所有量測都在 2026-10-14 03:00 前結束：方法 3 延遲 20 runs（含最後的 WCM W=1）、方法 2 16 runs、方法 3 容量搜尋 20 組（87 個 trial）。
  方法 1（wcm-tw128 sweep）在 capacity/runs/wcm-tw128/（22,250 EPS 被 OOM kill）。之後沒有再跑任何量測。
- 最終分析：`collect.py → finalize.py → plot.py`（`reproduce.sh analyze`）。輸出 method1_time_aligned.csv、
  method2_batch_volume.csv、method3_latency_5000eps.csv、method3_capacity.csv、clock_correction_audit.csv、charts/a–f。
- 所有延遲數字都是乾淨值或校正值：新 run 用 clock_drift.csv 校正；方法 1 的 tw128 用 stepdetect.py 偵測校正
  （舊 CSV 中 WCM W=1 = 7,750 ms 的原始污染值已改正為 1,206 ms／tw128 1,217 ms）；正式 sweep 經偵測確認窗內無階梯。
- W=1 容量低於正式值：批次設定等效、判準相同、容量搜尋期間沒有時鐘階梯 → 最可能是機器狀態加上 C1 對突發的敏感度，
  而且是單次 trial；WCM W=1 的上限是 OOM。詳見 README §4.2。兩組數字不混用。
- 結論與重現方式：見 README.md。
- 未做：FB Lua W=0.02／0.05 補點、80% 容量點延遲（extra）、在目前 session 用正式設定重跑容量以確認 W=1 落差的原因。

---

## 歷史紀錄（執行期間）

## 狀態
- 2026-10-13 18:3x 建立 harness（capacity_align.sh / run_point.sh / run_queue.sh）與 conf/ 複本；dry-run 驗證 FB/Vector/Fluentd conf OK。
- 重要發現（讀正式資料時）：正式 5,000 EPS 延遲 sweep 中，Fluentd（chunk_limit_size 1m）與 Vector（max_bytes 1 MiB）
  其實是 **1 MiB 輸出 size-trigger 主導**（sink：Fluentd 498 POST/180 s、Vector 360 POST/180 s ≈ 每 0.36/0.5 s 一批），
  不是 1 s timer；只有 FB Lua/WASM 是純 1 s timer（181 POST/180 s）。WCM-b1 則是 1 MiB **輸入** size-trigger（880 POST，~409 筆/POST）。
  → 方法 3 的 W=1.0 點：FB Lua/WASM 延遲可重用正式資料；Fluentd/Vector 必須重跑（chunk 128m / max_bytes 128MiB）。
  → 容量（正式 cap_rate_* 手動重測）：Fluentd 用 chunk 128m + flush 1、Vector 52 POST/60 s（約 7 MB/POST）、FB 1 s
    → 都是時間主導，W=1.0 容量可重用；WCM 25,000 是 wcm-b1，需重測 tw128。

## 觀察紀錄
- 18:37 fbit-lua W=0.01 @5000：eval_trial PASS，但延遲階梯式增長（30 s 子窗 p50：17 ms → 1.6 s → 3.2 s → 4.8 s → 7.2 s），
  sink 只有 ~52 POST/s（不是 100/s），輸出 1,876/s < 2,000/s → **不穩定**。新增穩定性判準：
  stable = PASS 且 p99(120–150 s) ≤ 1.3×p99(30–60 s)+20 ms（collect.py 的 latency_stable 欄）。
  已在佇列尾加 fbit-lua W=0.02、0.05 找最小穩定值。
- 時鐘：clocksource=tsc（非 hyperv_clocksource_tsc_page），與正式 sweep 一樣以 SKIP_CLOCK_CHECK=1 執行。

- **18:50 重大問題：時鐘跳躍**。本次開機（06:18）後 WSL2 clocksource=tsc，wall clock 每 ~32.2 s 向前跳 +1.59~1.75 s
  （clock_drift_logger 實測）。gen 的 ts = start_wall + 排程偏移（不受影響），sink receive = time.Now()（受影響）
  → 原始延遲每 32 s 多 ~1.6 s，看起來像「延遲階梯式增長」。上面 fbit-lua W=0.01 的「不穩定」其實是這個假象。
  eval_trial.py 的 C2（backlog）也被 wall/monotonic 混用放寬約 3 s。
  處置：(1) 前 4 個 run 移到 runs_invalid_clockstep/（不採用）；(2) 每個 run 全程記錄 clock_drift.csv；
  (3) correct_latency.py 扣掉 D(receive)−D(gen_start)、窗口改在 monotonic 時間軸；(4) eval_trial_align.py
  （C1–C4 不變，只把 wall 時刻換成 monotonic）；原版判定另存 verdict_orig_eval.txt。
  驗證：wcm-b1 @5000 重跑，校正後子窗 p50≈155–157 ms、p99≈260–265 ms，與正式 158/264 ms 一致（30–60 s 子窗有一次尖峰 p99 723 ms）。
  因時間基準可能與正式資料不同，W=1.0 的延遲與容量點也全部在目前時鐘下重測（正式值列為 formal_* 對照欄）。
  無 sudo，無法把 clocksource 改回 hyperv_clocksource_tsc_page；**其他 agent 的延遲量測同樣受影響**。

## 佇列
1. queue_m3_lat.txt：wcm-b1 驗證 + 方法 3、5,000 EPS 延遲（21 runs，各約 4 分鐘）— 19:03 重新開始
2. queue_m2.txt：方法 2（15 runs）
3. 方法 3 容量粗搜尋（cap 模式，20 組 product×W，含 W=1.0 重測）
4. queue_last.txt：WCM W=1.0（tw128）延遲：執行前先檢查 agent A 的 runs/wcm-tw128/
5. （有時間）各設定 80% 容量點延遲

## 執行方式
- run_queue.sh queue_m3_lat.txt（背景，18:31 起）→ run_rest.sh（等前者結束後：queue_m2.txt → 16 組容量粗搜尋，WCM W=1.0 最後）
- 每個 run 都在 flock /home/ckt1022/.wcm-bench.lock 內（含 container 啟停與分析）。建立 STOP 檔可在下一個 run 前停下。

## 已完成 runs
（見 logs/runs.log，每行一個 run）
