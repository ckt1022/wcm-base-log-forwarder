//! Direct Memory ABI benchmark 的簡易統計工具。
//!
//! 本檔案提供中位數與 nearest-rank 百分位數計算，供 host 摘要各 batch 的耗時。

/// 對輸入排序後計算中位數；偶數筆資料取中央兩值平均。
pub fn median(values: &[u128]) -> f64 {
    let mut values: Vec<u128> = values.to_vec();
    values.sort_unstable();
    let middle = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[middle - 1] as f64 + values[middle] as f64) / 2.0
    } else {
        values[middle] as f64
    }
}

/// 使用 nearest-rank 方法回傳指定比例的百分位數。
pub fn percentile(values: &[u128], fraction: f64) -> u128 {
    let mut values: Vec<u128> = values.to_vec();
    values.sort_unstable();
    let index = ((values.len() as f64 * fraction).ceil() as usize)
        .saturating_sub(1)
        .min(values.len() - 1);
    values[index]
}
