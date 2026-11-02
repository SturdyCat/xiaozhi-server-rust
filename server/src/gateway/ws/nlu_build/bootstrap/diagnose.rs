//! 失败归因与运维文案。
//!
//! 单独一个文件是因为这两件事的读者不同：归因决定**状态机怎么走**（能不能重试），
//! 文案决定**运维下一步查什么**。混在编排逻辑里最容易写成"原因一堆、没人知道怎么办"。
//!
//! 每条文案必须能回答"该重建 / 该查模型 / 该查环境"，这是硬约束。

/// 失败归因。顺序有意义：先排除"环境没准备好"，再谈"训练/产物本身"。
pub(super) fn classify_failure(error: &str, log: &str) -> &'static str {
    if error.contains("cannot start Docker CLI") {
        return "docker_unavailable";
    }
    if log.contains("Dependency image missing")
        || error.contains("cannot inspect trainer dependency image")
    {
        return "image_missing";
    }
    if error.contains("exceeded 3600 seconds") {
        return "timeout";
    }
    let lower = log.to_ascii_lowercase();
    if lower.contains("out of memory") || lower.contains("oom-kill") || lower.contains("killed") {
        return "oom";
    }
    // 旧镜像没有该臂权重时的真实形态：HF 拉取失败（网络或不存在），日志里是这些词。
    // 只看 exit code 会把"语料错误/OOM"也报成权重缺失，把人引去重建镜像。
    for marker in [
        "oserror",
        "connectionerror",
        "can't load",
        "couldn't connect",
        "localentrynotfound",
        "max retries exceeded",
        "connection reset",
        "nodename nor servname",
    ] {
        if lower.contains(marker) {
            return "weights_unavailable";
        }
    }
    // 顺序有意：`candidate labels do not match …` 同时含两个关键词，
    // "标签不一致"比"产物损坏"更能指导运维（前者是目录变了，后者是文件坏了）。
    if error.contains("labels") {
        return "labels_mismatch";
    }
    if error.contains("integrity mismatch")
        || error.contains("candidate")
        || error.contains("manifest")
    {
        return "artifact_invalid";
    }
    "build_failed"
}

/// 运维下一步该做什么。`kind` 取门禁/采用的全部失败码。
pub(super) fn next_step(kind: &str) -> &'static str {
    match kind {
        "smoke_bad_class" => {
            "该重建：有句子被接受却判成别的技能，会路由到错误技能。\
             先看 bad_class_detail 判断是语料歧义还是模型退化，修语料/模板后重新构建。"
        }
        "smoke_positive" => {
            "该查模型：模板展开句命中率不到 90%，多为训练或 int8 量化未收敛。\
             可换底座重建，或先人工测试候选再决定。"
        }
        "smoke_negative" => {
            "该查模型：训练语料内的闲聊句被当成技能接受，属功能性缺陷（训练或量化坏了）。\
             不要靠调阈值掩盖。"
        }
        "smoke_truncated" => {
            "该查脚本：冒烟用例被日志预算裁剪，无法完整验证。\
             检查 train.py 的 smoke 行长度（应远小于 2000 字节）或缩短模板。"
        }
        "smoke_timeout" => {
            "该查环境：冒烟推理 60 秒内没跑完，疑似 CPU 被争抢或模型过大。\
             核对宿主机负载后重新构建。"
        }
        "smoke_missing" => {
            "该查脚本：训练日志里没有可解析的冒烟用例（脚本未输出、日志被截断或数据缺失）。\
             按未通过处理，候选留盘。"
        }
        "model_load_failed" => {
            "该查产物：ONNX 加载失败。文件哈希是对的但跑不起来，多为导出或量化阶段坏了。"
        }
        "threshold_changed" => {
            "该查配置：候选阈值/标签映射与构建快照不一致。自动采用不替用户改判定条件，请手工核对。"
        }
        "labels_mismatch" => {
            "该重建：候选标签序与当前技能目录不一致，模型输出会与技能错位。按最新配置重新构建。"
        }
        "artifact_invalid" => {
            "该重建：候选未通过完整性校验（文件缺失/哈希不符/推理异常）。候选已留盘，未启用。"
        }
        "config_invalid" => {
            "该查配置：候选本身没问题，但写入配置被拒绝（合并或校验失败）。修正配置后手工启用候选。"
        }
        "config_no_persistence" => {
            "该查环境：服务以内置默认配置启动，没有持久化落点，无法启用候选。"
        }
        "adopt_io_error" => {
            "该查环境：配置库读写瞬时失败（SQLite busy / 磁盘抖动），已重试 3 次。\
             稍后手工启用候选，不必重建。"
        }
        "docker_unavailable" => {
            "该查环境：宿主找不到 docker CLI 或 docker.sock，训练无法启动。挂载 docker.sock 后重启。"
        }
        "image_missing" => {
            "该查环境：训练依赖镜像缺失且拉取失败。先推镜像再发服务（见部署顺序说明）。"
        }
        "weights_unavailable" => {
            "该查环境：镜像内没有该底座权重且 HF 拉取失败。重建训练镜像（权重烘进镜像）后重试。"
        }
        "build_slot_taken_over" => {
            "该查环境：训练槽位被手动构建接管，自举这一版的结果无法确认（**不是训练失败**）。\
             稍后自动重试；若反复被抢，先让手动构建跑完再让自举入场。"
        }
        "timeout" => "该查环境：训练超过 3600 秒被终止，检查宿主机负载或语料规模。",
        "oom" => "该查环境：训练进程被 OOM kill，检查容器内存上限与语料规模。",
        _ => "该查日志：未归类的失败，见 last_error。",
    }
}

/// 按显示宽度截断（不切坏多字节字符），末尾加省略号。
pub(super) fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut cut = max;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &text[..cut])
}

#[cfg(test)]
#[path = "diagnose_tests.rs"]
mod tests;
