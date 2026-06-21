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

    s2_anchor_crossing(planet_abs);
    s3_anchor_space_warp(planet_abs);
}

/// S3: ワープのアンカー間移動（媒介変数・厳密到着）。
///
/// S2 の知見：道中（2.5 AU）は f32 ローカルで持てない。ゆえにワープ中の権威位置は
/// *アンカー空間 f64* で媒介変数評価する（ADR-0022 の smoothstep / floored duration /
/// exact-arrival をそのまま流用）。各 tick で「最寄りアンカー＋f32 オフセット」へ射影して
/// クライアントへ渡す想定だが、本ステップでは到着の厳密性と道中の桁落ち非発生を確認する。
///
/// 検証：恒星近傍 (+8 km) から惑星近傍 (-1.2 km 手前) へワープ。到着点が計画到着点に
///       厳密一致（< 1 mm）し、道中の各 tick 位置が f64 媒介で滑らか（ulp 落ちなし）。
fn s3_anchor_space_warp(planet_abs: [f64; 3]) {
    println!("\n=== S3 — anchor-space parametric warp (exact arrival) ===");
    const WARP_SPEED_MPS: f64 = 3.0e9; // スパイク用の見かけワープ速度（m/s）
    const MIN_TICKS: u32 = 20;

    let start = [8_000.0_f64, 0.0, 0.0]; // 恒星近傍
    let arrival = [planet_abs[0] - 1_200.0, planet_abs[1] + 300.0, planet_abs[2]]; // 惑星近傍

    let warp_dist = dist3(start, arrival);
    let total = ((warp_dist / WARP_SPEED_MPS).ceil().max(0.0) as u32).max(MIN_TICKS);
    println!("warp distance = {warp_dist:.3e} m, floored duration = {total} ticks");

    let smoothstep = |t: f64| t * t * (3.0 - 2.0 * t);

    // 道中を f64 媒介で歩く。各 tick の位置と、前 tick からの変位が単調・連続かを見る。
    let mut prev = start;
    let mut min_step = f64::INFINITY;
    let mut max_step = 0.0_f64;
    let mut pos = start;
    for elapsed in 1..=total {
        let t = smoothstep(elapsed as f64 / total as f64);
        pos = [
            start[0] + (arrival[0] - start[0]) * t,
            start[1] + (arrival[1] - start[1]) * t,
            start[2] + (arrival[2] - start[2]) * t,
        ];
        let step = dist3(prev, pos);
        min_step = min_step.min(step);
        max_step = max_step.max(step);
        prev = pos;
    }

    let arrival_err = dist3(pos, arrival);
    println!("per-tick step: min {min_step:.3e} m, max {max_step:.3e} m (smoothstep ease: small at ends, large mid)");
    println!("final position vs planned arrival: err = {arrival_err:.6} m");

    // 到着点を惑星アンカーの f32 オフセットで持てるか（S2 の到着リベース）。
    let (arr_repr_err, arr_off_mag) = representation_error(arrival, planet_abs);
    println!("arrival held under planet anchor: offset mag {arr_off_mag:.1} m, repr err {arr_repr_err:.6} m");

    let pass = arrival_err < 1e-3 && arr_repr_err < 1.0;
    println!(
        "\nGATE S3 (exact arrival < 1 mm, arrival representable under dest anchor < 1 m): {}",
        if pass { "PASS" } else { "FAIL" }
    );
}

/// 真の絶対位置（f64）を、与えたアンカー基準の f32 オフセットで表現したときの
/// 往復誤差（= 表現がどれだけ真値からズレるか）。アンカーが近いほど小さい。
fn representation_error(truth_abs: [f64; 3], anchor: [f64; 3]) -> (f64, f32) {
    let off = [
        (truth_abs[0] - anchor[0]) as f32,
        (truth_abs[1] - anchor[1]) as f32,
        (truth_abs[2] - anchor[2]) as f32,
    ];
    let mag = off.iter().map(|c| c * c).sum::<f32>().sqrt();
    let rep = AnchorRelative { anchor_abs: anchor, offset: off };
    (dist3(rep.absolute(), truth_abs), mag)
}

/// S2: アンカー跨ぎリベース（B-2）＋ 真距離復元（B-3）。
///
/// 真値は f64 で持ち、各アンカー基準の f32 表現が真値からどれだけズレるかを測る。
/// 検証：
///   B-2  着弾点（惑星近傍）で「恒星アンカー基準 → 惑星アンカー基準」へリベースすると、
///        真値に対する誤差が ulp 級（数十 km）から < 1 mm に*回復*し、リベースで
///        絶対位置が飛ばない（恒星基準の劣化表現を経由しても、最終表現は真値に一致）。
///   B-3  別アンカー下の 2 点から星系内の真距離を桁落ちなく復元できる。
///
/// ついでに方式 B の本質的制約も露わにする：アンカー間 5 AU の*中間*は、どちらの
/// アンカー基準でも offset ≈ 2.5 AU（f32 ulp ≈ 16 km）で持てない。ゆえにワープ
/// *道中*の精度は f32 ローカルでは保てず、媒介変数をアンカー空間（f64/比率）で
/// 評価する必要がある（→ S3）。リベースは「到着時（offset 小）」に行うのが要件。
fn s2_anchor_crossing(planet_abs: [f64; 3]) {
    println!("\n=== S2 — anchor-crossing rebase (B-2) & global distance (B-3) ===");
    let star_abs = [0.0_f64, 0.0, 0.0];

    // 真値（f64）：惑星のすぐ近く（-1.2 km 手前, +300 m）で停止した着弾点。
    let arrival_truth = [planet_abs[0] - 1_200.0, planet_abs[1] + 300.0, planet_abs[2]];

    // 同じ真値を、恒星アンカー基準 / 惑星アンカー基準でそれぞれ f32 表現したときの誤差。
    let (err_under_star, mag_star) = representation_error(arrival_truth, star_abs);
    let (err_under_planet, mag_planet) = representation_error(arrival_truth, planet_abs);
    println!(
        "arrival point represented under STAR anchor:   offset mag {mag_star:.3e} m, err vs truth = {err_under_star:.1} m"
    );
    println!(
        "arrival point represented under PLANET anchor: offset mag {mag_planet:.1} m, err vs truth = {err_under_planet:.6} m"
    );
    println!(
        "  -> rebasing star->planet at arrival recovers precision by ~{:.0}x",
        (err_under_star.max(1e-9)) / err_under_planet.max(1e-9)
    );

    // ワープ道中（中間 2.5 AU）はどちらのアンカーでも f32 で持てないことを示す。
    let mid_truth = [planet_abs[0] * 0.5, 0.0, 0.0];
    let (err_mid_star, mag_mid) = representation_error(mid_truth, star_abs);
    println!(
        "MID-transit (2.5 AU) under either anchor: offset mag {mag_mid:.3e} m, err vs truth = {err_mid_star:.0} m  (=> transit precision is S3's job, parametric in anchor space)"
    );

    // B-3: 別アンカー下の 2 点（恒星近傍の船 X / 惑星近傍の船 Y）の真距離を復元。
    let ship_x_truth = [5_000.0, 0.0, 0.0];
    let ship_x = AnchorRelative {
        anchor_abs: star_abs,
        offset: [5_000.0, 0.0, 0.0],
    };
    let ship_y = AnchorRelative {
        anchor_abs: planet_abs,
        offset: [
            (arrival_truth[0] - planet_abs[0]) as f32,
            (arrival_truth[1] - planet_abs[1]) as f32,
            (arrival_truth[2] - planet_abs[2]) as f32,
        ],
    };
    let sep_via_anchors = dist3(ship_x.absolute(), ship_y.absolute());
    let truth_sep = dist3(ship_x_truth, arrival_truth);
    println!(
        "global distance star-ship<->planet-ship: via anchors = {sep_via_anchors:.3} m, truth = {truth_sep:.3} m (err {:.6} m)",
        (sep_via_anchors - truth_sep).abs()
    );

    // B-2 判定：着弾点を惑星アンカーで持てば真値誤差 < 1 m（恒星アンカーでは数十 km）。
    // B-3 判定：別アンカー 2 点の真距離復元誤差 < 1 m。
    let pass = err_under_planet < 1.0 && (sep_via_anchors - truth_sep).abs() < 1.0;
    println!(
        "\nGATE B-2/B-3 (arrival under dest anchor err < 1 m, global dist err < 1 m): {}",
        if pass { "PASS" } else { "FAIL" }
    );
    println!(
        "FINDING: anchors must be local at arrival (offset small); 2 anchors 5 AU apart cannot\n         hold the midpoint in f32 -- warp transit must be parametric in anchor space (S3)."
    );
}
