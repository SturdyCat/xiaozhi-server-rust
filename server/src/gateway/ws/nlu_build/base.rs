//! 预训练底座臂表：规范名 → hub 与微调超参。
//!
//! 刻意只支持**固定几臂**（不做通用模型注册表）：每一臂都是人工验证过的组合，
//! 用户可选空间必须与"镜像里预热了哪些权重"一致，否则会出现"选得到、但离线训不了"。
//!
//! 与 `tools/nlu-trainer/train.py` 的 `BASES` 表是**双表**关系：键集合、hub、lr、epochs
//! 逐字段由 `nlu_build/tests.rs` 的纯内存断言锁定（解析 `include_str!` 进来的脚本源码），
//! 防止只改一边导致"服务端日志说 h288、实际训的是 rbt3"。
//!
//! `DEFAULT_BASE_MODEL` 必须等于 `train.py` 的 `BASES` **首项**（同样有单测兜着）：
//! 它是"服务端缺省臂"，而脚本的 `BASE_NAME` 缺省值是 standalone `docker run` 的缺省臂，
//! 两者必须同批改，否则脱离服务端单跑脚本会得到另一个底座。

use serde_json::{json, Value};

/// 预训练底座。
#[derive(Clone, Copy, Debug)]
pub(crate) struct BaseModel {
    /// 规范名：请求体、manifest、状态回包与训练脚本的 `MODEL_NAME` 都只用它。
    pub(crate) name: &'static str,
    /// 实际 hub 仓库；服务端只用于日志排障，真正的下载地址在 `train.py` 的 BASES 表里。
    pub(crate) hub: &'static str,
    /// 微调学习率；经 `-e LEARNING_RATE=` 传给训练脚本。
    pub(crate) learning_rate: f64,
    /// 训练轮数；经 `-e EPOCHS=` 传给训练脚本。
    pub(crate) epochs: f64,
    /// int8 ONNX 产物的**估算**体积（MB），只用于界面提示，不参与任何校验。
    pub(crate) onnx_estimate_mb: u32,
    /// 界面说明：只说体积/延迟这类可验证的差异，不承诺准确率。
    pub(crate) note: &'static str,
}

/// 臂表顺序即界面展示顺序；首项是缺省臂，便于"默认项排在最前"的一致性。
pub(crate) const BASE_MODELS: [BaseModel; 2] = [
    // 缺省臂：6 层 miniRBT（h288 词表），体积与延迟更低，不代表更准。
    BaseModel {
        name: "minirbt-h288",
        hub: "hfl/minirbt-h288",
        learning_rate: 1e-4,
        epochs: 10.0,
        onnx_estimate_mb: 10,
        note: "6 层 miniRBT（h288 词表），更小更快",
    },
    // 兼容臂：3 层中文 RoBERTa，int8 后约 40MB，历史产物同源。
    BaseModel {
        name: "rbt3",
        hub: "hfl/rbt3",
        learning_rate: 5e-5,
        epochs: 6.0,
        onnx_estimate_mb: 40,
        note: "3 层中文 RoBERTa，约 40MB，兼容历史产物",
    },
];

/// 服务端缺省臂。改这里必须同批改 `train.py` 的 `BASE_NAME` 缺省值与前端 view 初值。
pub(crate) const DEFAULT_BASE_MODEL: &str = "minirbt-h288";
/// 底座来源：请求显式指定 / 缺省回落到默认臂。非法名字两种来源都要报 400，禁止静默回退。
pub(crate) const BASE_SOURCE_REQUEST: &str = "request";
pub(crate) const BASE_SOURCE_DEFAULT: &str = "default";

impl BaseModel {
    /// 传给训练容器的三个 env，`-e KEY=VALUE` **成对**。数值走 f64 Display
    /// （0.0001 / 0.00005 / 10 / 6），十进制、无指数记法，Python 侧 `float()` 直读，
    /// 日志里也可直接比对。
    ///
    /// `-e` 旗标必须由本函数携带：`docker run` 会把裸 `KEY=VALUE` 当成**镜像引用**——
    /// 2026-11-02 NAS 实测 `invalid reference format: repository name
    /// (library/MODEL_NAME=minirbt-h288) must be lowercase`，exit 125，构建秒败。
    /// 回归锁的是含 `-e` 的完整 docker 参数，不是裸值（见 tests.rs 同名契约）。
    pub(crate) fn env_args(self) -> [String; 6] {
        [
            "-e".into(),
            format!("MODEL_NAME={}", self.name),
            "-e".into(),
            format!("LEARNING_RATE={}", self.learning_rate),
            "-e".into(),
            format!("EPOCHS={}", self.epochs),
        ]
    }
}

/// 按规范名解析底座；`None` = 缺省臂（`DEFAULT_BASE_MODEL`）。
///
/// 未知名**不回落**到默认臂，也不当成 hub id：训练一次几分钟到几十分钟，
/// 悄悄换底座比直接报错代价高得多。
///
/// 名字先 trim 再查表，与 `train.py` 的 `os.environ.get(...).strip()` 对齐：
/// 两侧对 `" rbt3"` 的口径必须一致，否则前端拼出空格时服务端 400、脚本却能跑。
pub(crate) fn resolve_base(requested: Option<&str>) -> Result<(BaseModel, &'static str), String> {
    let (name, source) = match requested {
        Some(name) => (name.trim(), BASE_SOURCE_REQUEST),
        None => (DEFAULT_BASE_MODEL, BASE_SOURCE_DEFAULT),
    };
    BASE_MODELS
        .iter()
        .copied()
        .find(|base| base.name == name)
        .map(|base| (base, source))
        .ok_or_else(|| {
            let allowed = BASE_MODELS
                .iter()
                .map(|base| base.name)
                .collect::<Vec<_>>()
                .join(", ");
            format!("unknown base_model {name:?}; allowed: {allowed}")
        })
}

/// `GET /api/nlu/model/build` 的 `bases` 字段：界面选择器的**唯一数据源**。
///
/// 客户端不再硬编码档位（旧版本服务端没有该字段时才回落到本地两档），
/// 这样"服务端支持哪些臂"只有一处权威。
pub(crate) fn bases_json() -> Value {
    Value::Array(
        BASE_MODELS
            .iter()
            .map(|base| {
                json!({
                    "name": base.name,
                    "hub": base.hub,
                    "onnx_estimate_mb": base.onnx_estimate_mb,
                    "note": base.note,
                    "default": base.name == DEFAULT_BASE_MODEL,
                })
            })
            .collect(),
    )
}
