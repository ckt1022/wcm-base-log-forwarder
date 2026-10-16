wit_bindgen::generate!({
    path: "../wit",
    world: "benchmark",
});

struct MarkerBenchmark;

#[inline(always)]
fn mix(acc: u64, index: u32) -> u64 {
    acc.rotate_left(7)
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .wrapping_add(index as u64)
}

impl Guest for MarkerBenchmark {
    fn baseline(iterations: u32, seed: u64) -> u64 {
        let mut acc = seed;
        for index in 0..iterations {
            // 避免release在迴圈內部沒有運算而移除，添加一個可驗證的計算方式。
            acc = mix(acc, index);
        }
        acc
    }

    fn with_noop_sign(iterations: u32, seed: u64) -> u64 {
        let mut acc = seed;
        for index in 0..iterations {
            noop_sign();
            acc = mix(acc, index);
        }
        acc
    }

    fn with_time_sign(iterations: u32, seed: u64) -> u64 {
        let mut acc = seed;
        for index in 0..iterations {
            time_sign();
            acc = mix(acc, index);
        }
        acc
    }
}

export!(MarkerBenchmark);
