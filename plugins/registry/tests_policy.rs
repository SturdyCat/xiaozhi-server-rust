use super::super::*;

/// 指令闸门（`[command]`）的注册表语义护栏。
///
/// 它有两个"必须"：① 默认不选中（加插件不许改变现有部署行为）；
/// ② 启用后必须**新会话生效**且**字段档位有出处**（否则管理页永远显示"需重启"，属静默不正确）。
/// 坏配置（启用但词表为空）只能降级、不能拖垮语音服务，因此也不能是必需能力。
#[test]
fn command_gate_is_optional_and_hot_reloadable() {
    let mut cfg = Config::default();
    // 默认已改为启用（产品决策：标准行为、开机即用）；显式关闭=回到旧行为，仍是可选能力。
    assert!(cfg.command.enabled, "指令闸门默认必须启用");
    assert_eq!(active_impl(&cfg, Capability::Command), Some("command.gate"));
    assert!(
        !Capability::Command.required() && !Capability::Command.fatal_if_enabled(),
        "可选能力：坏配置只降级，不阻止启动"
    );

    cfg.command.enabled = false;
    assert_eq!(active_impl(&cfg, Capability::Command), None);
    cfg.command.enabled = true;
    assert_eq!(active_impl(&cfg, Capability::Command), Some("command.gate"));
    let d = find("command.gate").expect("应注册 command.gate");
    (d.validate)(&cfg).unwrap_or_else(|e| panic!("默认词表应通过校验: {e:#}"));
    assert_eq!(d.hot, HotReload::NextSession);
    // 字段级档位：改 enabled/keywords 都是"新会话生效"（`POST /api/config` 的 hot 据此判定）
    assert_eq!(
        field_hot(&["command", "enabled"]),
        Some(HotReload::NextSession)
    );
    assert_eq!(
        field_hot(&["command", "keywords"]),
        Some(HotReload::NextSession)
    );
    // 旧长度/匹配开关仅兼容读入，不再作为可调识别字段暴露。
    assert_eq!(field_hot(&["command", "max_chars"]), None);
    assert_eq!(field_hot(&["command", "match_mode"]), None);
    assert_eq!(section_hot("command"), HotReload::NextSession);

    // 启用但词表为空 = 闸门不生效：必须能被说成"配置问题"（而不是静默不拦）
    cfg.command.keywords.clear();
    let err = (d.validate)(&cfg).unwrap_err().to_string();
    assert!(err.contains("keywords"), "{err}");

    // schema 必须把这张卡的字段暴露出去（管理页「指令」页签的数据源）；
    // pure-base 不投影业务能力，该断言仅在 legacy 构建下成立。
    #[cfg(not(feature = "pure-base"))]
    {
        let v = schema_json();
        let cmd = v["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == "command")
            .expect("schema 应包含 command 能力");
        let keys: Vec<&str> = cmd["implementations"][0]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|f| f["key"].as_str())
            .collect();
        assert_eq!(keys, vec!["enabled", "keywords", "reply"], "{keys:?}");
    }
}

/// 模型唯一 NLU 的配置意图与运行状态分离；目录默认存在不等于模型已启用。
/// 坏目录拒绝保存，模型缺依赖不得回落规则，字段档位仍由注册表声明。
#[test]
fn nlu_model_is_optional_and_hot_reloadable() {
    let mut cfg = Config::default();
    // 默认已改为启用（产品决策：标准行为、开机即用）；显式关闭=回到旧行为，仍是可选能力。
    assert!(cfg.nlu.enabled, "规则 NLU 默认必须启用");
    assert_eq!(active_impl(&cfg, Capability::Nlu), Some("nlu.model"));
    assert!(
        !Capability::Nlu.required() && !Capability::Nlu.fatal_if_enabled(),
        "可选能力：坏配置只降级，不阻止启动"
    );

    // 显式关闭后不选中；再打开即选中；默认无意图应通过校验（不静默失效）。
    cfg.nlu.enabled = false;
    assert_eq!(active_impl(&cfg, Capability::Nlu), None);
    cfg.nlu.enabled = true;
    assert_eq!(active_impl(&cfg, Capability::Nlu), Some("nlu.model"));
    let d = find("nlu.model").expect("应注册 nlu.model");
    assert_eq!(find("nlu.rules").map(|legacy| legacy.id), Some(d.id));
    assert_eq!(
        impls_of(Capability::Nlu).count(),
        1,
        "兼容别名不得新增规则实现"
    );
    assert!(!cfg.nlu.bert_enabled, "默认技能种子不得自动打开未验收模型");
    (d.validate)(&cfg).unwrap_or_else(|e| panic!("默认技能种子应通过校验: {e:#}"));
    assert_eq!(d.hot, HotReload::NextSession);
    // 字段级档位：enabled 是"新会话生效"（POST /api/config 的 hot 据此判定）
    assert_eq!(field_hot(&["nlu", "enabled"]), Some(HotReload::NextSession));
    assert_eq!(section_hot("nlu"), HotReload::NextSession);

    // 启用但意图模板为空 = 永远命中不了：必须能被说成"配置问题"（而不是静默不拦）
    cfg.nlu.intents.push(crate::plugins::nlu::IntentDef {
        name: "control_light".into(),
        ..Default::default()
    });
    let err = (d.validate)(&cfg).unwrap_err().to_string();
    assert!(err.contains("templates"), "{err}");
    // 补上模板后即通过
    cfg.nlu
        .intents
        .last_mut()
        .unwrap()
        .templates
        .push("查看帮助".into());
    (d.validate)(&cfg).unwrap_or_else(|e| panic!("补上模板后应通过: {e:#}"));

    // schema 必须把这张卡的顶层字段暴露出去（管理页「NLU」页签的数据源）；
    // pure-base 不投影业务能力，该断言仅在 legacy 构建下成立。
    #[cfg(not(feature = "pure-base"))]
    {
        let v = schema_json();
        let nlu = v["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == "nlu")
            .expect("schema 应包含 nlu 能力");
        let keys: Vec<&str> = nlu["implementations"][0]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|f| f["key"].as_str())
            .collect();
        // intents/slots 为嵌套数组，不登记为扁平字段（见 fields_nlu.rs 说明）；
        // bert_* 为 TinyBERT 语义分类的扁平字段（label_map 仍是嵌套 TOML 高级配置）。
        assert_eq!(
            keys,
            vec![
                "enabled",
                "bert_enabled",
                "bert_model",
                "bert_vocab",
                "bert_labels",
                "bert_threshold",
                "bert_num_threads",
                "bert_max_len",
            ],
            "{keys:?}"
        );
    }
}

/// P1-7 漂移护栏：`[memory].engine` 的合法取值必须与注册表 memory.* 实现一一对应
/// （外加 "none"）。新增后端变体（如 P1-9 的 rag）若忘了挂注册表或漏改
/// `MEMORY_ENGINE_VALUES`/schema 字段，本测试直接打红——三处必须同源同步。
#[test]
fn memory_backend_values_match_registry_impls() {
    use crate::plugins::memory::config::MEMORY_ENGINE_VALUES;

    // (1) 硬约束：当前注册表的 memory 实现（动态收集，防止改名后断言失真）。
    let impl_ids: Vec<String> = impls_of(Capability::Memory)
        .map(|d| d.id.to_string())
        .collect();
    assert_eq!(
        impl_ids,
        vec!["memory.graph", "memory.store", "memory.rag"],
        "memory 实现清单漂移：新增后端须挂注册表并同步此断言"
    );

    // (2) serde 合法取值 = 实现名去能力前缀 + "none"。
    let from_impls: Vec<String> = impl_ids
        .iter()
        .map(|id| id.trim_start_matches("memory.").to_string())
        .chain(["none".to_string()])
        .collect();
    let from_enum: Vec<String> = MEMORY_ENGINE_VALUES.iter().map(|s| s.to_string()).collect();
    assert_eq!(
        from_enum, from_impls,
        "MEMORY_ENGINE_VALUES 与注册表实现漂移"
    );

    // (3) schema 的 engine Enum 字段取值 = MEMORY_ENGINE_VALUES（单一出处对账）。
    let d = find("memory.graph").expect("应注册 memory.graph");
    let engine_field = d
        .fields
        .iter()
        .find(|f| f.key == "engine")
        .expect("memory.graph 应有 engine 字段");
    let FieldKind::Enum(values) = engine_field.kind else {
        panic!("engine 字段应为 Enum");
    };
    assert_eq!(
        values.len(),
        MEMORY_ENGINE_VALUES.len(),
        "schema Enum 与常量取值数漂移"
    );
    for (a, b) in values.iter().zip(MEMORY_ENGINE_VALUES.iter()) {
        assert_eq!(*a, *b, "schema Enum 与常量取值漂移");
    }

    // (4) serde 行为抽查：每个合法值都能反序列化，且序列化值回环一致。
    for v in MEMORY_ENGINE_VALUES {
        let cfg: crate::plugins::memory::MemoryConfig =
            toml::from_str(&format!("engine = \"{v}\"")).expect(v);
        let serialized = serde_json::to_string(&cfg.engine).unwrap();
        assert_eq!(
            serialized.as_str(),
            format!("\"{v}\""),
            "engine 序列化应回环: {v}"
        );
    }
}

/// 唤醒问候语（`[wakeup]`）的注册表语义护栏。
///
/// 与指令闸门同构，但多一条**跨段矛盾**：`persist` 复用经内存 TTS 缓存承载成品帧，
/// 因此 `[tts].cache_entries = 0` 时落盘会**静默失效**（写不进去、下次还得重新合成）。
/// 这条必须被拦下并说清修法——静默失效是本仓库最忌讳的失败模式。
#[test]
fn wake_greeting_is_optional_hot_reloadable_and_flags_cache_conflict() {
    let mut cfg = Config::default();
    // 默认已改为启用（产品决策：唤醒问候是「开机即用」的标准体验）；显式关闭=回到旧行为。
    assert!(cfg.wakeup.enabled, "唤醒问候语默认必须启用");
    assert_eq!(active_impl(&cfg, Capability::Wake), Some("wakeup.greeting"));
    assert!(
        !Capability::Wake.required() && !Capability::Wake.fatal_if_enabled(),
        "可选能力：坏配置只降级，不阻止启动"
    );

    cfg.wakeup.enabled = false;
    assert_eq!(active_impl(&cfg, Capability::Wake), None);
    cfg.wakeup.enabled = true;
    assert_eq!(active_impl(&cfg, Capability::Wake), Some("wakeup.greeting"));
    let d = find("wakeup.greeting").expect("应注册 wakeup.greeting");
    (d.validate)(&cfg).unwrap_or_else(|e| panic!("默认问候语应通过校验: {e:#}"));
    assert_eq!(d.hot, HotReload::NextSession);
    // 字段级档位：三个字段都改变行为，都必须是"新会话生效"
    for k in ["enabled", "greeting", "persist"] {
        assert_eq!(
            field_hot(&["wakeup", k]),
            Some(HotReload::NextSession),
            "[wakeup].{k} 的档位必须有出处（否则管理页永远显示「需重启」）"
        );
    }
    assert_eq!(section_hot("wakeup"), HotReload::NextSession);

    // 启用但问候语为空 = 不问候：必须能被说成"配置问题"（而不是静默不出声）
    cfg.wakeup.greeting = "   ".to_string();
    let err = (d.validate)(&cfg).unwrap_err().to_string();
    assert!(err.contains("greeting"), "{err}");

    // 跨段矛盾：persist 依赖内存缓存承载成品帧，缓存关掉时落盘静默失效 → 必须拦下并给修法
    cfg.wakeup.greeting = "我在".to_string();
    cfg.tts.cache_entries = 0;
    let err = (d.validate)(&cfg).unwrap_err().to_string();
    assert!(err.contains("cache_entries"), "要指出真正的矛盾源: {err}");
    assert!(err.contains("persist"), "要给出修法（关掉 persist）: {err}");
    // 关掉 persist 或恢复缓存后即可通过（两者任一都能修好，不能只认一种）
    cfg.wakeup.persist = false;
    (d.validate)(&cfg).unwrap_or_else(|e| panic!("关掉 persist 后应通过: {e:#}"));
    cfg.wakeup.persist = true;
    cfg.tts.cache_entries = 256;
    (d.validate)(&cfg).unwrap_or_else(|e| panic!("恢复缓存后应通过: {e:#}"));

    // schema 必须把这张卡的字段暴露出去（管理页「唤醒」页签的数据源）；
    // pure-base 不投影业务能力，该断言仅在 legacy 构建下成立。
    #[cfg(not(feature = "pure-base"))]
    {
        let v = schema_json();
        let wake = v["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == "wakeup")
            .expect("schema 应包含 wakeup 能力");
        let keys: Vec<&str> = wake["implementations"][0]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|f| f["key"].as_str())
            .collect();
        assert_eq!(keys, vec!["enabled", "greeting", "persist"], "{keys:?}");
    }
}

/// `[llm].enabled` 总开关的注册表语义护栏：默认 true（缺省零行为变化）、
/// 字段必须暴露给管理页且档位为 Live（保存后下一轮生效）。
///
/// 为什么不是 NextSession：它是 ASR→TTS 链路自测开关，而 EchoMuse 设备的会话整场
/// 不断线（同一个 session 跨多轮），等"新会话"等于永不生效——实机复现过"配置里关了
/// LLM，设备仍连续多轮照常调云端"。
#[test]
fn llm_enabled_switch_is_live_and_projected() {
    assert!(Config::default().llm.enabled, "llm.enabled 默认必须为 true");
    assert_eq!(
        field_hot(&["llm", "enabled"]),
        Some(HotReload::Live),
        "llm.enabled 是链路自测开关：按轮生效（respond 现读 live_config）"
    );
    let v = schema_json();
    let llm = v["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "llm")
        .expect("schema 应包含 llm 能力");
    let common = llm["common_fields"]
        .as_array()
        .expect("llm 应有公共字段表")
        .iter()
        .flat_map(|g| g["fields"].as_array().unwrap().clone())
        .filter_map(|f| f["key"].as_str().map(str::to_string))
        .collect::<Vec<_>>();
    assert!(
        common.iter().any(|k| k == "enabled"),
        "管理页必须能看到 llm.enabled 开关，实际公共字段: {common:?}"
    );
}

/// 备用链写时校验：不完整条目（缺 api_base 或 model）在保存/启动期即被拒绝，
/// 且报错指名第几条（对齐 DSH「写时就拒绝，指名道姓」；不学 hermes 的静默忽略）。
#[test]
fn llm_fallback_incomplete_provider_is_rejected() {
    let d = REGISTRY
        .iter()
        .find(|d| d.capability == Capability::Llm)
        .expect("llm 描述符");
    let mut cfg = Config::default();
    cfg.llm.api_base = "https://api.example.com/v1/responses".into();
    cfg.llm.fallback.providers = vec![crate::plugins::llm::LlmFallbackProvider {
        api_base: String::new(),
        api_key: "k".into(),
        model: "m".into(),
    }];
    let err = (d.validate)(&cfg).unwrap_err().to_string();
    assert!(
        err.contains("llm.fallback.providers") && err.contains("第 1 条"),
        "报错应指名备用条目: {err}"
    );

    cfg.llm.fallback.providers[0].api_base = "https://backup.example.com/v1/responses".into();
    (d.validate)(&cfg).unwrap_or_else(|e| panic!("完整备用条目应通过: {e:#}"));

    // 密钥为空的备用是**软性**问题：可启动，但状态必须 Degraded 提示（不静默）。
    cfg.llm
        .fallback
        .providers
        .push(crate::plugins::llm::LlmFallbackProvider {
            api_base: "https://backup2.example.com/v1/responses".into(),
            api_key: String::new(),
            model: "m2".into(),
        });
    let warn = (d.warn_if.expect("llm 应有 warn_if"))(&cfg).expect("空密钥应提示");
    assert!(
        warn.contains("第 2 条备用"),
        "warn 应指名空密钥的备用条目: {warn}"
    );
}

// ============================================================
// asr.wasm-remote（P4b）：选择 / 构建 / 依赖文案
// ============================================================

fn wasm_asr_descriptor() -> &'static crate::plugins::registry::PluginDescriptor {
    crate::plugins::registry::REGISTRY
        .iter()
        .find(|d| d.id == "asr.wasm-remote")
        .expect("asr.wasm-remote 应在注册表")
}

#[test]
fn wasm_asr_engine_selection_matches_engine_id() {
    let mut cfg = Config::default();
    cfg.asr.engine = "wasm-remote".into();
    let d = wasm_asr_descriptor();
    assert!(d.is_selected(&cfg));
    // 同能力互斥：sensevoice 此时不再选中
    let sensevoice = crate::plugins::registry::REGISTRY
        .iter()
        .find(|d| d.id == "asr.sensevoice")
        .unwrap();
    assert!(!sensevoice.is_selected(&cfg));
    // 默认引擎仍是 sensevoice（零影响）
    let dflt = Config::default();
    assert!(sensevoice.is_selected(&dflt));
    assert!(!wasm_asr_descriptor().is_selected(&dflt));
}

#[cfg(not(feature = "wasm-plugins"))]
#[test]
fn wasm_asr_engine_build_requires_feature() {
    let mut cfg = Config::default();
    cfg.asr.engine = "wasm-remote".into();
    let err = match (wasm_asr_descriptor().build.unwrap())(&cfg) {
        Ok(_) => panic!("未编入 wasm-plugins 时不应构建成功"),
        Err(err) => err,
    };
    assert!(
        err.to_string().contains("wasm-plugins"),
        "缺 feature 的文案应给修复指引：{err}"
    );
}

#[cfg(feature = "wasm-plugins")]
#[test]
fn wasm_asr_engine_descriptor_builds_late_bound() {
    use crate::plugins::registry::Built;

    let mut cfg = Config::default();
    cfg.asr.engine = "wasm-remote".into();
    let built = (wasm_asr_descriptor().build.unwrap())(&cfg).expect("wasm feature 下构建成功");
    let Built::Asr(_engine) = built else {
        panic!("应为 Asr 构建");
    };
    // 真实宿主注入与 wasm 往返由 wasm_engine_tests 覆盖；本测试不触碰全局晚绑槽，避免
    // 与并行测试的全局宿主/首选项相互污染。
}

// ============================================================
// tts.wasm-policy（P4）：WASM 控制策略 + native PCM
// ============================================================

fn wasm_tts_policy_descriptor() -> &'static crate::plugins::registry::PluginDescriptor {
    crate::plugins::registry::REGISTRY
        .iter()
        .find(|d| d.id == "tts.wasm-policy")
        .expect("tts.wasm-policy 应在注册表")
}

#[test]
fn wasm_tts_policy_selection_matches_engine_id() {
    let mut cfg = Config::default();
    cfg.tts.engine = "wasm-policy".into();
    let d = wasm_tts_policy_descriptor();
    assert!(d.is_selected(&cfg));
    let kokoro = crate::plugins::registry::REGISTRY
        .iter()
        .find(|d| d.id == "tts.kokoro")
        .unwrap();
    assert!(!kokoro.is_selected(&cfg));
    assert!(kokoro.is_selected(&Config::default()));
    assert!(!wasm_tts_policy_descriptor().is_selected(&Config::default()));
}

#[cfg(not(feature = "wasm-plugins"))]
#[test]
fn wasm_tts_policy_build_requires_feature() {
    let mut cfg = Config::default();
    cfg.tts.engine = "wasm-policy".into();
    let err = match (wasm_tts_policy_descriptor().build.unwrap())(&cfg) {
        Ok(_) => panic!("未编入 wasm-plugins 时不应构建成功"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("wasm-plugins"), "{err:#}");
}

#[cfg(feature = "wasm-plugins")]
#[test]
fn wasm_tts_policy_requires_native_sherpa_dependency() {
    let mut cfg = Config::default();
    cfg.tts.engine = "wasm-policy".into();
    let err = match (wasm_tts_policy_descriptor().build.unwrap())(&cfg) {
        Ok(_) => panic!("无 sherpa/native 模型时策略包装不应构建成功"),
        Err(err) => err,
    };
    assert!(
        err.to_string().contains("sherpa") || err.to_string().contains("模型"),
        "策略包装仍需 native Kokoro 依赖：{err:#}"
    );
}
