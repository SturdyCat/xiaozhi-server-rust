//! 预训练底座：**当前唯一支持 `minirbt-h288`**。
//!
//! 不再做多臂选择——单模型即默认值，请求层与状态层都不再出现 base_model 选择。
//! 保留 `BaseModel` 仅为了把"传给训练容器的三个 env"与 hub 排障信息集中在一处。
//!
//! 与 `tools/nlu-trainer/train.py` 的 `BASES` 表是**双表**关系：键集合、hub、lr、epochs
//! 逐字段由 `nlu_build/tests.rs` 的纯内存断言锁定（解析 `include_str!` 进来的脚本源码），
//! 防止只改一边导致"服务端日志说 h288、实际训的是别的"。
//!
//! `BASE_MODEL.name` 必须等于 `train.py` 的 `BASES` **首项**（同样有单测兜着）：
//! 它是"服务端唯一臂"，而脚本的 `BASE_NAME` 缺省值是 standalone `docker run` 的缺省臂，
//! 两者必须同批改，否则脱离服务端单跑脚本会得到另一个底座。

/// 预训练底座。
#[derive(Clone, Copy, Debug)]
pub(crate) struct BaseModel {
    /// 规范名：训练脚本的 `MODEL_NAME` 只使用它。
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

/// 唯一预训练底座：6 层 miniRBT（h288 词表），体积与延迟更低，不代表更准。
pub(crate) const BASE_MODEL: BaseModel = BaseModel {
    name: "minirbt-h288",
    hub: "hfl/minirbt-h288",
    learning_rate: 1e-4,
    epochs: 10.0,
    onnx_estimate_mb: 10,
    note: "6 层 miniRBT（h288 词表），更小更快",
};

impl BaseModel {
    /// 传给训练容器的三个 env，`-e KEY=VALUE` **成对**。数值走 f64 Display
    /// （0.0001 / 10.0），十进制、无指数记法，Python 侧 `float()` 直读，
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
