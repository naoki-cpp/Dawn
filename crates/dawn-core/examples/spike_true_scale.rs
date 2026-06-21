//! ADR-0028 真スケール座標スパイク — S1: ローカル f32 戦闘（ゲート B-1）
//!
//! **これは捨てコード**（throwaway spike）。本番には統合しない。`cargo run -p
//! dawn-core --example spike_true_scale` で数値を観測するためだけのもの。
//!
//! 検証したいこと（ADR-0028 §スパイク設計メモ B-1）：
//!   恒星=原点、外縁惑星を *実 AU*（5 AU ≈ 7.48e11 m）に置いたとき、惑星近傍で
//!   km スケールの戦闘運動が f32 で破綻しないか。鍵は「絶対グローバル f32 では
//!   ない・アンカー（惑星）相対の f32 オフセットで持つ」こと。
//!
//! 比較：
//!   (a) anchor-relative … 位置を「惑星アンカー＋f32 ローカルオフセット」で保持。
//!       積分はオフセット（小さい＝km 桁）に対して行う。
//!   (b) naive-global  … 位置を f32 の絶対座標（planet_abs + offset）で保持。
//!       7.48e11 では f32 ulp ≈ 65 km なので、数百 m/tick の運動が桁落ちで消える。
//!
//! 真値（ground truth）は f64 で並走させ、各方式の誤差を測る。
//! 期待：(a) の誤差 < 1 m、(b) は破綻（ulp 級の誤差・船が「動かない」）。

const AU_M: f64 = 1.495_978_707e11; // 1 天文単位（m）
const PLANET_AU: f64 = 5.0; // 外縁惑星の軌道半径（AU）
const TICKS: u32 = 600; // 観測 tick 数
const DT_S: f64 = 1.0; // 1 tick = 1 s（速度 m/s = m/tick）

/// アンカー相対表現：絶対の巨大座標は持たず、アンカー原点（ここでは惑星の真位置）
/// からの f32 ローカルオフセットだけを持つ。これがスパイクで試す方式 B の最小形。
#[derive(Clone, Copy)]
struct AnchorRelative {
    /// アンカーの真の絶対位置（m）。サーバ内部では i64/f64 定数や別レイヤーで持つ
    /// 想定。スパイクでは f64 定数で代用（型は本番で詰める・非目標）。
    anchor_abs: [f64; 3],
    /// アンカーからの f32 ローカルオフセット（m）。戦闘演算はこれに対して行う。
    offset: [f32; 3],
}

impl AnchorRelative {
    fn absolute(&self) -> [f64; 3] {
        [
            self.anchor_abs[0] + self.offset[0] as f64,
            self.anchor_abs[1] + self.offset[1] as f64,
            self.anchor_abs[2] + self.offset[2] as f64,
        ]
    }
    /// 速度（m/s）を 1 tick 積分。オフセット（小さい）に対して f32 で加算する。
    fn step(&mut self, vel_mps: [f32; 3]) {
        for i in 0..3 {
            self.offset[i] += vel_mps[i] * DT_S as f32;
        }
    }
}

fn dist3(a: [f64; 3], b: [f64; 3]) -> f64 {
    let mut s = 0.0;
    for i in 0..3 {
        let d = a[i] - b[i];
        s += d * d;
    }
    s.sqrt()
}

fn main() {
    let planet_abs = [PLANET_AU * AU_M, 0.0, 0.0];
    println!("=== ADR-0028 spike S1 — local f32 combat near a real-AU planet ===");
    println!(
        "planet anchor at {:.3} AU = {:.3e} m   (f32 ulp here ~ {:.0} m)",
        PLANET_AU,
        planet_abs[0],
        // f32 ulp at x ≈ 2^(floor(log2 x) - 23)
        2f64.powi((planet_abs[0].log2().floor() as i32) - 23)
    );

    // 2 隻のダミー船を惑星近傍に置く：~30 km 離して対向させる（戦闘間合い）。
    // 真値は f64、anchor-relative は f32 オフセット、naive-global は f32 絶対。
    let start_off = [
        [20_000.0_f64, 0.0, 0.0], // ship A: 惑星 +20 km
        [-10_000.0_f64, 5_000.0, 0.0], // ship B: 惑星 -10 km, +5 km
    ];
    let vel = [
        [-300.0_f32, 50.0, 0.0], // A は -X へ 300 m/s で接近
        [120.0_f32, -40.0, 0.0], // B は +X へ 120 m/s
    ];

    // 真値（f64 絶対）
    let mut truth = [[0.0_f64; 3]; 2];
    // anchor-relative（f32 offset）
    let mut anchored = [AnchorRelative { anchor_abs: planet_abs, offset: [0.0; 3] }; 2];
    // naive-global（f32 絶対）
    let mut naive = [[0.0_f32; 3]; 2];

    for s in 0..2 {
        for i in 0..3 {
            truth[s][i] = planet_abs[i] + start_off[s][i];
            anchored[s].offset[i] = start_off[s][i] as f32;
            naive[s][i] = (planet_abs[i] + start_off[s][i]) as f32;
        }
    }

    let mut max_off_mag = 0.0_f32; // オフセット最大値（f32 安全域 1e7 を超えないか）

    for _t in 0..TICKS {
        for s in 0..2 {
            // 真値
            for i in 0..3 {
                truth[s][i] += vel[s][i] as f64 * DT_S;
            }
            // anchor-relative
            anchored[s].step(vel[s]);
            let mag = anchored[s]
                .offset
                .iter()
                .map(|c| c * c)
                .sum::<f32>()
                .sqrt();
            max_off_mag = max_off_mag.max(mag);
            // naive-global
            for i in 0..3 {
                naive[s][i] += vel[s][i] * DT_S as f32;
            }
        }
    }

    println!("\nafter {TICKS} ticks ({} s, dt={} s):", TICKS as f64 * DT_S, DT_S);
    for s in 0..2 {
        let name = if s == 0 { "A" } else { "B" };
        let err_anchor = dist3(anchored[s].absolute(), truth[s]);
        let naive_abs = [naive[s][0] as f64, naive[s][1] as f64, naive[s][2] as f64];
        let err_naive = dist3(naive_abs, truth[s]);
        let moved_truth = dist3(truth[s], [
            planet_abs[0] + start_off[s][0],
            planet_abs[1] + start_off[s][1],
            planet_abs[2] + start_off[s][2],
        ]);
        println!(
            "ship {name}: truth moved {moved_truth:8.1} m | anchor-rel err = {err_anchor:10.3} m | naive-global err = {err_naive:12.1} m"
        );
    }

    // ペア間距離（戦闘間合い）の誤差も見る — 命中判定に効くのは相対距離。
    let truth_sep = dist3(truth[0], truth[1]);
    let anchor_sep = dist3(anchored[0].absolute(), anchored[1].absolute());
    let naive_sep = dist3(
        [naive[0][0] as f64, naive[0][1] as f64, naive[0][2] as f64],
        [naive[1][0] as f64, naive[1][1] as f64, naive[1][2] as f64],
    );
    println!("\npair separation (combat range):");
    println!("  truth        = {truth_sep:10.3} m");
    println!("  anchor-rel   = {anchor_sep:10.3} m  (err {:.3} m)", (anchor_sep - truth_sep).abs());
    println!("  naive-global = {naive_sep:10.3} m  (err {:.1} m)", (naive_sep - truth_sep).abs());

    println!("\nmax local offset magnitude = {max_off_mag:.1} m  (f32 safe to ~1e7 m: {})",
        if max_off_mag < 1e7 { "OK" } else { "OVER" });

    // ゲート B-1 判定
    let pass = {
        let a0 = dist3(anchored[0].absolute(), truth[0]);
        let a1 = dist3(anchored[1].absolute(), truth[1]);
        a0 < 1.0 && a1 < 1.0 && (anchor_sep - truth_sep).abs() < 1.0 && max_off_mag < 1e7
    };
    println!("\nGATE B-1 (anchor-rel pos err < 1 m, sep err < 1 m, offset < 1e7): {}",
        if pass { "PASS" } else { "FAIL" });
}
