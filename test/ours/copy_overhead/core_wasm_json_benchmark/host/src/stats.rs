//! 基準測試結果的簡易統計工具。
//!
//! 本檔案提供中位數與百分位數計算，供主程式摘要各批次的總耗時分布。

/// 對輸入值排序後計算中位數；偶數筆資料取中央兩值的平均。
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

/// 使用 nearest-rank 方法計算指定比例的百分位數。
pub fn percentile(values: &[u128], fraction: f64) -> u128 {
    let mut values: Vec<u128> = values.to_vec();
    values.sort_unstable();
    let index = ((values.len() as f64 * fraction).ceil() as usize)
        .saturating_sub(1)
        .min(values.len() - 1);
    values[index]
}
