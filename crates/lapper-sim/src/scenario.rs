use lapper_core::{Config, Course, GateConfig, GateId, MILLISECOND, Micros, SECOND};

/// シミュレーションの条件。
#[derive(Debug, Clone)]
pub struct Scenario {
    pub name: &'static str,
    pub course: Vec<GateId>,
    /// 区間ごとの標準的なタイム（秒）。
    pub segment_times: Vec<f64>,
    /// 計測ロジックに設定する最小区間タイム（標準タイムに対する割合）。
    pub min_segment_ratio: f64,
    /// 計測ロジックに設定する最大区間タイム（標準タイムに対する割合）。`None` なら設定しない。
    pub max_segment_ratio: Option<f64>,
    /// 車ごとの速さのばらつき（標準偏差、割合）。
    pub driver_spread: f64,
    /// 区間ごとのタイムのばらつき（標準偏差、割合）。
    pub run_jitter: f64,
    pub cars: u32,
    /// 1 台あたりの走行回数。
    pub heats: u32,
    /// スタートの最小間隔（秒）。
    pub start_interval: f64,
    /// 同時にコース上にいられる台数。スターターはこれを守り、計測ロジックにも設定する。
    pub max_on_course: usize,
    /// 同じゲートの通過どうしの最小間隔（秒）。スターターはこれを守ってスタートを遅らせる。
    pub min_gap: f64,
    /// ゲートごとのセンサーの反応の遅れ。打刻にはこれが加わり、計測ロジックの設定で補正する。
    pub delays: Vec<(GateId, Micros)>,
    pub faults: Faults,
}

/// 注入する障害。確率は 0〜1。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Faults {
    /// 通過 1 回ごとの、光電管が反応しない確率。
    pub miss: f64,
    /// 通過 1 回ごとの、直後にもう一度反応する（二重反応）確率。
    pub chatter: f64,
    /// 1 時間あたりの、車と関係ない誤反応の回数。
    pub noise_per_hour: f64,
    /// 走行 1 回ごとの、コースアウトする確率。
    pub course_out: f64,
    /// コースアウトを報告する確率。
    pub report: f64,
    /// 出走待ちの列に入らずにスタートする確率。
    pub unqueued: f64,
    /// 列の順と入れ替わってスタートする確率（隣の車と）。
    pub swap: f64,
    /// 車の取り違え（列に入っていない、入れ替わり）を運営が訂正する確率。
    pub fix: f64,
}

impl Faults {
    pub const NONE: Faults = Faults {
        miss: 0.0,
        chatter: 0.0,
        noise_per_hour: 0.0,
        course_out: 0.0,
        report: 0.0,
        unqueued: 0.0,
        swap: 0.0,
        fix: 0.0,
    };

    pub const LIGHT: Faults = Faults {
        miss: 0.005,
        chatter: 0.05,
        noise_per_hour: 1.0,
        course_out: 0.03,
        report: 0.8,
        unqueued: 0.02,
        swap: 0.02,
        fix: 0.9,
    };

    pub const HEAVY: Faults = Faults {
        miss: 0.03,
        chatter: 0.3,
        noise_per_hour: 6.0,
        course_out: 0.1,
        report: 0.5,
        unqueued: 0.05,
        swap: 0.05,
        fix: 0.5,
    };

    pub fn by_name(name: &str) -> Option<Faults> {
        match name {
            "none" => Some(Faults::NONE),
            "light" => Some(Faults::LIGHT),
            "heavy" => Some(Faults::HEAVY),
            _ => None,
        }
    }
}

pub const PRESETS: [&str; 3] = ["gymkhana", "shared", "dirt"];

impl Scenario {
    pub fn preset(name: &str, faults: Faults) -> Option<Scenario> {
        let (s, i1, i2, g) = (GateId(0), GateId(1), GateId(2), GateId(3));
        let base = Scenario {
            name: "gymkhana",
            course: vec![s, i1, i2, g],
            segment_times: vec![20.0, 25.0, 20.0],
            min_segment_ratio: 0.6,
            max_segment_ratio: None,
            driver_spread: 0.06,
            run_jitter: 0.02,
            cars: 30,
            heats: 2,
            start_interval: 16.0,
            max_on_course: 4,
            min_gap: 3.0,
            delays: vec![(g, 8 * MILLISECOND)],
            faults,
        };
        match name {
            // スタート / 中間×2 / ゴール、最大 4 台の同時走行
            "gymkhana" => Some(base),
            // スタートとゴールが同じ光電管で、1 台ずつ走る練習会
            "shared" => Some(Scenario {
                name: "shared",
                course: vec![s, s],
                segment_times: vec![60.0],
                min_segment_ratio: 0.7,
                max_segment_ratio: Some(1.5),
                cars: 20,
                start_interval: 10.0,
                max_on_course: 1,
                delays: vec![],
                ..base
            }),
            // スタートとゴールが離れた長いコース、最大 2 台
            "dirt" => Some(Scenario {
                name: "dirt",
                course: vec![s, i1, g],
                segment_times: vec![55.0, 65.0],
                cars: 40,
                start_interval: 45.0,
                max_on_course: 2,
                delays: vec![(i1, 3 * MILLISECOND), (g, 5 * MILLISECOND)],
                ..base
            }),
            _ => None,
        }
    }

    /// 1 走行の標準タイム（秒）。
    pub fn nominal_lap(&self) -> f64 {
        self.segment_times.iter().sum()
    }

    /// 計測ロジックの設定。
    pub fn config(&self) -> Config {
        let mut config = Config::new(Course::new(self.course.clone()).expect("valid course"));
        config.min_segment = self
            .segment_times
            .iter()
            .map(|&t| seconds(t * self.min_segment_ratio))
            .collect();
        if let Some(ratio) = self.max_segment_ratio {
            config.max_segment = self
                .segment_times
                .iter()
                .map(|&t| Some(seconds(t * ratio)))
                .collect();
        }
        config.max_on_course = Some(self.max_on_course);
        config.max_run_time = Some(seconds(self.nominal_lap() * 3.0));
        for &(gate, delay) in &self.delays {
            config.gates.insert(
                gate,
                GateConfig {
                    delay,
                    ..GateConfig::default()
                },
            );
        }
        config
    }

    pub fn delay(&self, gate: GateId) -> Micros {
        self.delays
            .iter()
            .find(|(g, _)| *g == gate)
            .map_or(0, |&(_, d)| d)
    }
}

pub fn seconds(t: f64) -> Micros {
    (t * SECOND as f64).round() as Micros
}
